//! Host `ChatModel` for the ChatGPT **Codex** backend (OpenAI OAuth login).
//!
//! An OpenAI provider authenticated with a ChatGPT-subscription OAuth token is
//! routed to `https://chatgpt.com/backend-api/codex/responses`
//! ([`super::openai_codex`]). That backend is stricter than the public API:
//!
//! - it **rejects** any request without `"stream": true` with
//!   `HTTP 400 {"detail":"Stream must be set to true"}`;
//! - it requires `"store": false` and an `instructions` string;
//! - it rejects `max_output_tokens` (and never sees `temperature` / `stop` /
//!   `seed` from the Codex CLI, so this client does not send them either).
//!
//! The vendored `tinyagents` `OpenAiModel` Responses path is a non-streaming,
//! text-only port (`stream` is never sent, the body is read as a single JSON
//! document, `function_call` output items are not decoded and tool results are
//! flattened to user text). Pointing it at the Codex backend therefore fails on
//! the first request of every turn. Per the `crate_openai` module contract,
//! `openai_codex` is a bespoke provider that stays host-side, so this module owns
//! the wire: it always streams, folds the SSE event stream into one
//! [`ModelResponse`] for the unary [`ChatModel::invoke`] path, and forwards the
//! deltas for [`ChatModel::stream`]. Tool calls travel as native
//! `function_call` / `function_call_output` items.
//!
//! The API-key OpenAI path is untouched: it never sets `responses_api_primary`
//! and keeps using the crate's Chat Completions model.
//!
//! Logging uses the `[providers][openai-codex]` prefix. Bearer tokens, request
//! bodies and message text are never logged.

use std::time::Duration;

use async_trait::async_trait;
use futures::channel::mpsc;
use serde_json::{json, Map, Value};
use tinyagents::harness::message::{ContentBlock, Message};
use tinyagents::harness::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
    ProviderError, ResponseFormat, ToolChoice,
};
use tinyagents::harness::providers::openai::OpenAiModel;
use tinyagents::{Result as TaResult, TinyAgentsError};

use super::openai_codex_sse::{fold_sse, make_provider_error, RecoverPolicy};

const LOG: &str = "[providers][openai-codex]";
/// Codex rejects a request without `instructions`; sent when the transcript has
/// no system message.
const FALLBACK_INSTRUCTIONS: &str = "You are a helpful assistant.";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Wait for the response headers (the backend answers as soon as the run starts).
const HEADER_TIMEOUT: Duration = Duration::from_secs(180);

/// Wire settings the factory resolved for the Codex backend.
pub(crate) struct CodexWireConfig<'a> {
    pub provider_name: &'a str,
    pub endpoint: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    pub headers: &'a [(String, String)],
    pub query_params: &'a [(String, String)],
    pub user_agent: Option<&'a str>,
}

/// Streaming Responses-API client for the Codex backend.
pub(crate) struct CodexResponsesModel {
    /// Supplies `profile()` / `cache_identity()` so capability and cache keys
    /// stay identical to the crate model this replaces on the wire.
    inner: OpenAiModel,
    client: reqwest::Client,
    provider: String,
    url: String,
    api_key: String,
    model: String,
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    user_agent: Option<String>,
    /// Whether tool schemas / calls travel natively. `false` keeps the
    /// prompt-guided dialect: no `tools`, tool results rendered as user text.
    native_tools: bool,
}

impl CodexResponsesModel {
    pub(crate) fn new(inner: OpenAiModel, config: CodexWireConfig<'_>) -> Self {
        let client = crate::neppy::util::tls::tls_client_builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|err| {
                log::warn!("{LOG} client build failed, using default client: {err}");
                reqwest::Client::new()
            });
        let base = config.endpoint.trim_end_matches('/');
        let url = if base.ends_with("/responses") {
            base.to_string()
        } else {
            format!("{base}/responses")
        };
        let native_tools = inner.native_tools_enabled();
        log::debug!(
            "{LOG} model built provider={} model={} url_host={} native_tools={native_tools}",
            config.provider_name,
            config.model,
            crate::neppy::inference::provider::factory::redact_endpoint(&url),
        );
        Self {
            inner,
            client,
            provider: config.provider_name.to_string(),
            url,
            api_key: config.api_key.to_string(),
            model: config.model.to_string(),
            headers: config.headers.to_vec(),
            query: config.query_params.to_vec(),
            user_agent: config.user_agent.map(str::to_string),
            native_tools,
        }
    }

    fn provider_error(
        &self,
        message: String,
        status: Option<u16>,
        code: Option<String>,
    ) -> ProviderError {
        make_provider_error(&self.provider, &self.model, message, status, code, None)
    }

    /// POSTs the streaming request and returns the checked (2xx) response.
    async fn post(&self, request: &ModelRequest) -> TaResult<reqwest::Response> {
        let model = request.model.clone().unwrap_or_else(|| self.model.clone());
        let body = build_request_body(&model, request, self.native_tools);
        // The backend has nothing to answer when every message was empty or
        // whitespace, and `input: []` only buys a round trip to an opaque 400.
        // Fail here, by name, and not retryably: resending the same empty
        // transcript cannot succeed.
        if body["input"].as_array().map_or(true, Vec::is_empty) {
            log::warn!(
                "{LOG} refusing to send an empty transcript model={model} messages={}",
                request.messages.len()
            );
            return Err(TinyAgentsError::from_provider_error(empty_input_error(
                &self.provider,
                &self.model,
            )));
        }
        log::debug!(
            "{LOG} request start model={model} stream=true input_items={} tools={}",
            body["input"].as_array().map_or(0, Vec::len),
            body["tools"].as_array().map_or(0, Vec::len),
        );
        let mut builder = self
            .client
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .json(&body);
        for (name, value) in &self.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some(user_agent) = &self.user_agent {
            builder = builder.header(reqwest::header::USER_AGENT, user_agent.as_str());
        }
        if !self.query.is_empty() {
            builder = builder.query(&self.query);
        }
        // `request.timeout_ms` bounds the WHOLE request: reqwest's per-request
        // timeout runs from connect until the response body has been read, so
        // it covers the SSE body and not only the wait for headers. The body's
        // own 300s idle timeout (`fold_sse`) still applies inside it. Without
        // a caller deadline only the header wait and the idle timeout bound it.
        let total = request.timeout_ms.map(Duration::from_millis);
        if let Some(total) = total {
            builder = builder.timeout(total);
        }
        let wait = total.unwrap_or(HEADER_TIMEOUT);
        log::debug!(
            "{LOG} deadlines model={model} header_wait_s={} whole_request_s={:?}",
            wait.as_secs(),
            total.map(|t| t.as_secs())
        );
        let response = match tokio::time::timeout(wait, builder.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                log::warn!("{LOG} transport failure model={model}: {err}");
                let code = err.is_timeout().then(|| "timeout".to_string());
                return Err(TinyAgentsError::from_provider_error(self.provider_error(
                    format!("codex responses request failed: {err}"),
                    None,
                    code,
                )));
            }
            Err(_) => {
                log::warn!("{LOG} timed out waiting for response headers model={model}");
                return Err(TinyAgentsError::from_provider_error(self.provider_error(
                    format!(
                        "codex responses request timed out after {}s",
                        wait.as_secs()
                    ),
                    None,
                    Some("timeout".to_string()),
                )));
            }
        };
        let status = response.status();
        log::debug!("{LOG} response status={} model={model}", status.as_u16());
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let error = parse_error_body(&self.provider, &self.model, status.as_u16(), &text);
            log::warn!(
                "{LOG} request rejected status={} code={:?} message={}",
                status.as_u16(),
                error.code,
                truncate(&error.message, 300)
            );
            return Err(TinyAgentsError::from_provider_error(error));
        }
        Ok(response)
    }
}

#[async_trait]
impl ChatModel<()> for CodexResponsesModel {
    fn profile(&self) -> Option<&ModelProfile> {
        ChatModel::<()>::profile(&self.inner)
    }

    fn cache_identity(&self) -> Option<String> {
        ChatModel::<()>::cache_identity(&self.inner)
    }

    async fn invoke(&self, _state: &(), request: ModelRequest) -> TaResult<ModelResponse> {
        let response = self.post(&request).await?;
        let recover = self.native_tools_recovery(&request);
        fold_sse(response, &self.provider, &self.model, recover, &mut |_| {
            true
        })
        .await
    }

    async fn stream(&self, _state: &(), request: ModelRequest) -> TaResult<ModelStream> {
        let response = self.post(&request).await?;
        let recover = self.native_tools_recovery(&request);
        let (tx, rx) = mpsc::unbounded::<ModelStreamItem>();
        let (provider, model) = (self.provider.clone(), self.model.clone());
        let _ = tx.unbounded_send(ModelStreamItem::Started);
        tokio::spawn(async move {
            let mut sink = |item: ModelStreamItem| tx.unbounded_send(item).is_ok();
            let terminal = match fold_sse(response, &provider, &model, recover, &mut sink).await {
                Ok(done) => ModelStreamItem::Completed(done),
                Err(TinyAgentsError::Provider(err)) => ModelStreamItem::ProviderFailed(*err),
                Err(other) => ModelStreamItem::Failed(other.to_string()),
            };
            let _ = tx.unbounded_send(terminal);
        });
        Ok(Box::pin(rx))
    }
}

impl CodexResponsesModel {
    /// Whether the folded response should have `<tool_call>` blocks recovered
    /// (prompt-guided models, the crate's `should_recover` rule).
    fn native_tools_recovery(&self, request: &ModelRequest) -> RecoverPolicy {
        RecoverPolicy {
            native: self.native_tools,
            has_tools: !request.tools.is_empty(),
        }
    }
}

/// The error for a transcript that produced no `input` items.
///
/// Built directly rather than through [`make_provider_error`], which would
/// classify it from the message text: this one is never retryable.
fn empty_input_error(provider: &str, model: &str) -> ProviderError {
    ProviderError {
        provider: provider.to_string(),
        model: Some(model.to_string()),
        status: None,
        code: Some("invalid_request".to_string()),
        message: "codex responses request has no input: every message was empty or whitespace"
            .to_string(),
        retryable: false,
        retry_after_ms: None,
        raw: None,
    }
}

/// Decodes a non-2xx body. The Codex backend answers `{"detail": "..."}`; the
/// public API answers `{"error": {"message", "code"|"type"}}`.
pub(super) fn parse_error_body(
    provider: &str,
    model: &str,
    status: u16,
    text: &str,
) -> ProviderError {
    let raw = serde_json::from_str::<Value>(text).ok();
    let error_obj = raw.as_ref().and_then(|v| v.get("error"));
    let message = error_obj
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .or_else(|| {
            raw.as_ref()
                .and_then(|v| v.get("message"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            raw.as_ref()
                .and_then(|v| v.get("detail"))
                .and_then(Value::as_str)
        })
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(text)
        .to_string();
    let code = error_obj
        .and_then(|e| e.get("code").or_else(|| e.get("type")))
        .and_then(Value::as_str)
        .map(str::to_string);
    make_provider_error(provider, model, message, Some(status), code, raw)
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((idx, _)) => format!("{}…", &text[..idx]),
        None => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Request body
// ---------------------------------------------------------------------------

/// Builds the Codex `/responses` body. Always `stream: true` and
/// `store: false`; never `max_output_tokens`, `temperature`, `top_p`, `seed` or
/// `stop`.
pub(super) fn build_request_body(model: &str, request: &ModelRequest, native_tools: bool) -> Value {
    let (instructions, input) = build_input(&request.messages, native_tools);
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("instructions".into(), json!(instructions));
    body.insert("input".into(), Value::Array(input));
    body.insert("stream".into(), json!(true));
    body.insert("store".into(), json!(false));

    if native_tools && !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                    "strict": false,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
        body.insert(
            "tool_choice".into(),
            match &request.tool_choice {
                ToolChoice::Auto => json!("auto"),
                ToolChoice::None => json!("none"),
                ToolChoice::Required => json!("required"),
                ToolChoice::Tool(name) => json!({ "type": "function", "name": name }),
            },
        );
    }
    if let Some(reasoning) = request.reasoning.as_ref().filter(|r| !r.is_empty()) {
        let mut object = Map::new();
        if let Some(effort) = reasoning.effort {
            object.insert("effort".into(), json!(effort.as_str()));
        }
        if let Some(summary) = &reasoning.summary {
            object.insert("summary".into(), json!(summary));
        }
        if !object.is_empty() {
            body.insert("reasoning".into(), Value::Object(object));
        }
    }
    if let Some(format) = request.response_format.as_ref().and_then(text_format) {
        body.insert("text".into(), json!({ "format": format }));
    }
    Value::Object(body)
}

fn text_format(format: &ResponseFormat) -> Option<Value> {
    match format {
        ResponseFormat::Text => None,
        ResponseFormat::JsonObject => Some(json!({ "type": "json_object" })),
        ResponseFormat::JsonSchema { name, schema } | ResponseFormat::Auto { name, schema } => {
            Some(json!({ "type": "json_schema", "name": name, "schema": schema, "strict": false }))
        }
    }
}

fn blocks_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(ContentBlock::as_text)
        .collect::<Vec<_>>()
        .join("")
}

fn tool_output_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.clone()),
            ContentBlock::Json(value) => Some(value.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn user_item(role: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "role": role,
        "content": [{ "type": "input_text", "text": text }],
    })
}

/// Splits the transcript into Codex `instructions` and Responses `input` items.
fn build_input(messages: &[Message], native_tools: bool) -> (String, Vec<Value>) {
    let mut instructions = Vec::new();
    let mut input = Vec::new();
    for message in messages {
        match message {
            Message::System(m) => {
                let text = blocks_text(&m.content);
                if !text.trim().is_empty() {
                    instructions.push(text);
                }
            }
            Message::User(m) => {
                let mut parts: Vec<Value> = Vec::new();
                for block in &m.content {
                    match block {
                        ContentBlock::Text(text) if !text.trim().is_empty() => {
                            parts.push(json!({ "type": "input_text", "text": text }));
                        }
                        ContentBlock::Image(image) => {
                            parts.push(json!({ "type": "input_image", "image_url": image.url }));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    input.push(json!({ "type": "message", "role": "user", "content": parts }));
                }
            }
            Message::Assistant(m) => {
                let text = blocks_text(&m.content);
                if !text.trim().is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{ "type": "output_text", "text": text }],
                    }));
                }
                if native_tools {
                    for call in &m.tool_calls {
                        let arguments = match &call.arguments {
                            Value::String(raw) => raw.clone(),
                            Value::Null => "{}".to_string(),
                            other => other.to_string(),
                        };
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call.id,
                            "name": call.name,
                            "arguments": arguments,
                        }));
                    }
                }
            }
            Message::Tool(m) => {
                let output = tool_output_text(&m.content);
                if native_tools {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": m.tool_call_id,
                        "output": output,
                    }));
                } else if !output.trim().is_empty() {
                    input.push(user_item(
                        "user",
                        &format!("[tool_result id={}]\n{output}", m.tool_call_id),
                    ));
                }
            }
        }
    }
    let instructions = if instructions.is_empty() {
        FALLBACK_INSTRUCTIONS.to_string()
    } else {
        instructions.join("\n\n")
    };
    (instructions, input)
}

#[cfg(test)]
#[path = "openai_codex_model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "openai_codex_model_deadline_tests.rs"]
mod deadline_tests;
