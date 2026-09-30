//! SSE framing and event folding for the Codex Responses stream
//! ([`super::openai_codex_model`]).
//!
//! Pure, synchronous and network-free apart from [`fold_sse`], so the whole
//! event grammar is unit-testable from JSON fixtures.

use std::collections::HashMap;
use std::time::Duration;

use futures::StreamExt;
use serde_json::{json, Value};
use tinyagents::harness::message::{AssistantMessage, ContentBlock, MessageDelta};
use tinyagents::harness::model::{ModelResponse, ModelStreamItem, ProviderError};
use tinyagents::harness::retry::classify_provider_failure;
use tinyagents::harness::tool::{ToolCall, ToolDelta};
use tinyagents::harness::usage::Usage;
use tinyagents::{Result as TaResult, TinyAgentsError};

const LOG: &str = "[providers][openai-codex]";
/// Longest silence tolerated between two SSE chunks before the call is failed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Whether the folded response should have `<tool_call>` blocks recovered
/// (prompt-guided models, the crate's `should_recover` rule).
#[derive(Clone, Copy)]
pub(super) struct RecoverPolicy {
    pub(super) native: bool,
    pub(super) has_tools: bool,
}

pub(super) fn make_provider_error(
    provider: &str,
    model: &str,
    message: String,
    status: Option<u16>,
    code: Option<String>,
    raw: Option<Value>,
) -> ProviderError {
    let retryable = classify_provider_failure(status, code.as_deref(), &message).is_retryable();
    ProviderError {
        provider: provider.to_string(),
        model: Some(model.to_string()),
        status,
        code,
        message,
        retryable,
        retry_after_ms: None,
        raw,
    }
}

/// Incremental Server-Sent-Events framer: bytes in, `data:` payloads out.
#[derive(Default)]
pub(super) struct SseDecoder {
    buf: Vec<u8>,
}

impl SseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some((end, sep)) = find_event_boundary(&self.buf) {
            let raw: Vec<u8> = self.buf.drain(..end + sep).collect();
            if let Some(data) = event_data(&raw[..end]) {
                out.push(data);
            }
        }
        out
    }

    /// Flushes a trailing event the server closed without a blank line.
    pub(super) fn finish(&mut self) -> Option<String> {
        let raw = std::mem::take(&mut self.buf);
        event_data(&raw)
    }
}

fn find_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    (0..buf.len()).find_map(|i| {
        if buf[i..].starts_with(b"\n\n") {
            Some((i, 2))
        } else if buf[i..].starts_with(b"\r\n\r\n") {
            Some((i, 4))
        } else {
            None
        }
    })
}

fn event_data(raw: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(raw);
    let mut data = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    (!data.trim().is_empty()).then_some(data)
}

#[derive(Default)]
pub(super) struct ResponseFolder {
    text: String,
    reasoning: String,
    /// Finished output items, in arrival order (authoritative).
    items: Vec<Value>,
    /// `item_id` → `call_id`, for routing argument deltas.
    item_calls: HashMap<String, String>,
    response: Option<Value>,
    finish_reason: Option<String>,
    pub(super) terminal: bool,
    events: usize,
}

impl ResponseFolder {
    /// Applies one decoded event; returns the stream items it produces, or the
    /// provider failure it reports.
    pub(super) fn apply(
        &mut self,
        event: &Value,
        provider: &str,
        model: &str,
    ) -> Result<Vec<ModelStreamItem>, ProviderError> {
        self.events += 1;
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let delta = || event.get("delta").and_then(Value::as_str).unwrap_or("");
        let mut out = Vec::new();
        match kind {
            "response.output_text.delta" if !delta().is_empty() => {
                self.text.push_str(delta());
                out.push(ModelStreamItem::MessageDelta(MessageDelta::text(delta())));
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta"
                if !delta().is_empty() =>
            {
                self.reasoning.push_str(delta());
                out.push(ModelStreamItem::MessageDelta(MessageDelta {
                    reasoning: delta().to_string(),
                    ..MessageDelta::default()
                }));
            }
            "response.output_item.added" => {
                let item = event.get("item").unwrap_or(&Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let call_id = str_field(item, "call_id");
                    if let Some(item_id) = item.get("id").and_then(Value::as_str) {
                        self.item_calls.insert(item_id.to_string(), call_id.clone());
                    }
                    out.push(ModelStreamItem::ToolCallDelta(ToolDelta {
                        call_id,
                        content: String::new(),
                        tool_name: Some(str_field(item, "name")).filter(|n| !n.is_empty()),
                    }));
                }
            }
            "response.function_call_arguments.delta" if !delta().is_empty() => {
                let item_id = event.get("item_id").and_then(Value::as_str).unwrap_or("");
                let call_id = self
                    .item_calls
                    .get(item_id)
                    .cloned()
                    .unwrap_or_else(|| item_id.to_string());
                out.push(ModelStreamItem::ToolCallDelta(ToolDelta {
                    call_id,
                    content: delta().to_string(),
                    tool_name: None,
                }));
            }
            "response.output_item.done" => {
                if let Some(item) = event.get("item") {
                    self.items.push(item.clone());
                }
            }
            "response.completed" | "response.done" => {
                self.response = event.get("response").cloned();
                self.terminal = true;
            }
            "response.incomplete" => {
                let reason = event
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("incomplete");
                self.finish_reason = Some(
                    if reason == "max_output_tokens" {
                        "length"
                    } else {
                        reason
                    }
                    .to_string(),
                );
                self.response = event.get("response").cloned();
                self.terminal = true;
            }
            "response.failed" => {
                let error = event.pointer("/response/error").unwrap_or(&Value::Null);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("response failed");
                let code = error
                    .get("code")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return Err(make_provider_error(
                    provider,
                    model,
                    message.to_string(),
                    None,
                    code,
                    Some(event.clone()),
                ));
            }
            "error" => {
                let error = event.get("error").unwrap_or(event);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("stream error");
                let code = error
                    .get("code")
                    .or_else(|| error.get("type"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return Err(make_provider_error(
                    provider,
                    model,
                    message.to_string(),
                    None,
                    code,
                    Some(event.clone()),
                ));
            }
            _ => {}
        }
        Ok(out)
    }

    pub(super) fn finish(self, policy: RecoverPolicy) -> ModelResponse {
        let items = if self.items.is_empty() {
            self.response
                .as_ref()
                .and_then(|r| r.get("output"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        } else {
            self.items
        };
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut signature = None;
        let mut tool_calls = Vec::new();
        for item in &items {
            match item.get("type").and_then(Value::as_str) {
                Some("message") => {
                    for part in item
                        .get("content")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        match part.get("type").and_then(Value::as_str) {
                            Some("output_text") | Some("refusal") => {
                                let piece = part.get("text").or_else(|| part.get("refusal"));
                                text.push_str(piece.and_then(Value::as_str).unwrap_or(""));
                            }
                            _ => {}
                        }
                    }
                }
                Some("reasoning") => {
                    for part in item
                        .get("summary")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(piece) = part.get("text").and_then(Value::as_str) {
                            if !reasoning.is_empty() {
                                reasoning.push('\n');
                            }
                            reasoning.push_str(piece);
                        }
                    }
                    if let Some(enc) = item.get("encrypted_content").and_then(Value::as_str) {
                        signature = Some(enc.to_string());
                    }
                }
                Some("function_call") => tool_calls.push(parse_function_call(item)),
                _ => {}
            }
        }
        if text.is_empty() {
            text = self.text;
        }
        if reasoning.is_empty() {
            reasoning = self.reasoning;
        }
        let usage = self
            .response
            .as_ref()
            .and_then(|r| r.get("usage"))
            .map(parse_usage);

        let mut content = Vec::new();
        if !reasoning.is_empty() {
            content.push(ContentBlock::Thinking {
                text: reasoning,
                signature,
            });
        }
        if !text.is_empty() || tool_calls.is_empty() {
            content.push(ContentBlock::Text(text));
        }
        let finish_reason = self.finish_reason.unwrap_or_else(|| {
            if tool_calls.is_empty() {
                "stop"
            } else {
                "tool_calls"
            }
            .to_string()
        });
        log::debug!(
            "{LOG} stream folded events={} items={} tool_calls={} finish={finish_reason}",
            self.events,
            items.len(),
            tool_calls.len(),
        );
        let response = ModelResponse {
            message: AssistantMessage {
                id: None,
                content,
                tool_calls,
                usage,
            },
            usage,
            finish_reason: Some(finish_reason),
            raw: self.response,
            resolved_model: None,
            continue_turn: None,
            served_from_cache: false,
        };
        if tinyagents::harness::tool::should_recover(
            policy.native,
            policy.has_tools,
            response.message.tool_calls.len(),
        ) {
            return tinyagents::harness::tool::apply_prompt_tool_calls(response);
        }
        response
    }
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn parse_function_call(item: &Value) -> ToolCall {
    let id = {
        let call_id = str_field(item, "call_id");
        if call_id.is_empty() {
            str_field(item, "id")
        } else {
            call_id
        }
    };
    let name = str_field(item, "name");
    let raw = item.get("arguments").and_then(Value::as_str).unwrap_or("");
    if raw.trim().is_empty() {
        return ToolCall {
            id,
            name,
            arguments: json!({}),
            invalid: None,
        };
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(arguments) => ToolCall {
            id,
            name,
            arguments,
            invalid: None,
        },
        Err(err) => {
            log::warn!("{LOG} tool call {name} has unparseable arguments: {err}");
            ToolCall {
                id,
                name,
                arguments: Value::String(raw.to_string()),
                invalid: Some(format!("arguments are not valid JSON: {err}")),
            }
        }
    }
}

fn parse_usage(usage: &Value) -> Usage {
    let num = |ptr: &str| usage.pointer(ptr).and_then(Value::as_u64).unwrap_or(0);
    let input_tokens = num("/input_tokens");
    let output_tokens = num("/output_tokens");
    Usage {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
        cache_read_tokens: num("/input_tokens_details/cached_tokens"),
        cache_creation_tokens: 0,
        reasoning_tokens: num("/output_tokens_details/reasoning_tokens"),
    }
}

/// Drives an SSE response to completion. `sink` receives each incremental item
/// and returns `false` when the consumer has gone away (the call is abandoned).
pub(super) async fn fold_sse(
    response: reqwest::Response,
    provider: &str,
    model: &str,
    policy: RecoverPolicy,
    sink: &mut (dyn FnMut(ModelStreamItem) -> bool + Send),
) -> TaResult<ModelResponse> {
    let mut decoder = SseDecoder::default();
    let mut folder = ResponseFolder::default();
    let mut body = response.bytes_stream();
    let fail = |message: String, code: Option<&str>| {
        TinyAgentsError::from_provider_error(make_provider_error(
            provider,
            model,
            message,
            None,
            code.map(str::to_string),
            None,
        ))
    };

    'read: loop {
        let chunk = match tokio::time::timeout(IDLE_TIMEOUT, body.next()).await {
            Err(_) => {
                log::warn!(
                    "{LOG} stream idle for {}s model={model}",
                    IDLE_TIMEOUT.as_secs()
                );
                return Err(fail("codex stream stalled".to_string(), Some("timeout")));
            }
            Ok(None) => break,
            Ok(Some(Err(err))) => {
                log::warn!("{LOG} stream read failed model={model}: {err}");
                // A whole-request deadline (`ModelRequest::timeout_ms`) fires
                // here as a reqwest timeout on the body; name it like the
                // idle timeout so callers classify both the same way.
                let code = err.is_timeout().then_some("timeout");
                return Err(fail(format!("codex stream read failed: {err}"), code));
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        for data in decoder.push(&chunk) {
            if !apply_data(&mut folder, &data, provider, model, sink)? {
                return Err(fail(
                    "codex stream consumer dropped".to_string(),
                    Some("cancelled"),
                ));
            }
            if folder.terminal {
                break 'read;
            }
        }
    }
    if !folder.terminal {
        if let Some(data) = decoder.finish() {
            apply_data(&mut folder, &data, provider, model, sink)?;
        }
    }
    if !folder.terminal {
        log::warn!("{LOG} stream ended without a terminal event model={model}");
        return Err(fail(
            "codex stream ended before response.completed".to_string(),
            Some("stream_truncated"),
        ));
    }
    Ok(folder.finish(policy))
}

/// Applies one `data:` payload. `Ok(false)` means the sink is closed.
fn apply_data(
    folder: &mut ResponseFolder,
    data: &str,
    provider: &str,
    model: &str,
    sink: &mut (dyn FnMut(ModelStreamItem) -> bool + Send),
) -> TaResult<bool> {
    if data.trim() == "[DONE]" {
        folder.terminal = true;
        return Ok(true);
    }
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        log::trace!("{LOG} skipping non-JSON SSE payload ({} bytes)", data.len());
        return Ok(true);
    };
    let items = folder
        .apply(&event, provider, model)
        .map_err(TinyAgentsError::from_provider_error)?;
    for item in items {
        if !sink(item) {
            return Ok(false);
        }
    }
    Ok(true)
}
