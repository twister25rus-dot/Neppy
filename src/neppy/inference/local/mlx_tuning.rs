//! Request defaults and text-tool recovery for `mlx:` chat models.
//!
//! Two things the OpenAI-compatible transport cannot do for a local MLX server
//! on its own, both applied by [`MlxTunedModel`] around the crate model:
//!
//! 1. **Sampling and thinking defaults.** Neppy sends only a temperature
//!    (0.3 to 0.4), so everything else falls to the server: greedy-ish decoding,
//!    `max_tokens` 2048 and an unbounded thinking block. A Qwen-family chat
//!    model wants roughly temperature 0.6, `top_p` 0.95 and `top_k` 20, and a
//!    reasoning answer needs more than 2048 tokens. [`MlxRequestDefaults`]
//!    resolves those from the `[[mlx.server]]` block: a value the user set is
//!    sent as-is, and the `-1` "unset" sentinel becomes the default.
//! 2. **Text-form tool calls.** With native tool calling on, a model that
//!    drifts from its template emits the call as `content`. See
//!    [`super::mlx_tool_text`].
//!
//! Fields are only ever *filled*: a value already on the request (set by a
//! per-turn control, a caller, or the crate) wins, exactly like
//! `TurnControlsChatModel`.
//!
//! `top_k`, `min_p` and `thinking_budget` travel as top-level request fields
//! through `provider_options`, which the OpenAI-compatible transport merges
//! into the body. `mlx_vlm.server` 0.7 declares all three on its chat request
//! (`top_k`, `min_p`, `thinking_budget`), so the thinking ceiling is enforced
//! server-side (a forced `</think>` once the budget is spent) and needs no host
//! counter. The server only counts thinking tokens when thinking is enabled for
//! the request (the block's `enable_thinking` or a per-turn `enable_thinking`),
//! so the budget is a no-op for a block with thinking off; this module does not
//! switch thinking on behind the user's back.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tinyagents::harness::message::{ContentBlock, MessageDelta};
use tinyagents::harness::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyagents::harness::tool::{next_synthetic_call_id, ToolCall};

use super::mlx_tool_text::{
    hold_from_for_bare_function, recover_text_tool_calls, KnownTools, Recovery,
};
use crate::neppy::config::schema::MlxServerConfig;
use crate::neppy::config::Config;

const LOG: &str = "[mlx:tuning]";

/// Qwen-family recommended sampling for chat with thinking on.
pub(crate) const DEFAULT_TEMPERATURE: f64 = 0.6;
pub(crate) const DEFAULT_TOP_P: f64 = 0.95;
pub(crate) const DEFAULT_TOP_K: i64 = 20;
/// Output cap when neither the request nor the block sets one. The server's own
/// default is 2048, which a reasoning model spends on thinking alone.
pub(crate) const DEFAULT_MAX_TOKENS: u32 = 8192;
/// What `thinking_budget = 0` ("auto") means per request.
pub(crate) const AUTO_THINKING_BUDGET: u32 = 4096;
/// Room left for the answer after a full thinking block.
const ANSWER_HEADROOM_TOKENS: u32 = 4096;

/// Defaults resolved from one `[[mlx.server]]` block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MlxRequestDefaults {
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: i64,
    /// `None` leaves the server's own (0.0, disabled).
    pub min_p: Option<f64>,
    pub max_tokens: u32,
    /// `None` when the server cannot take it (`lm` kind, speculative decoding).
    pub thinking_budget: Option<u32>,
}

impl MlxRequestDefaults {
    /// Resolve defaults for `server`; `None` (no block) behaves like a default
    /// `vlm` block.
    pub(crate) fn for_block(server: Option<&MlxServerConfig>) -> Self {
        let default_block;
        let server = match server {
            Some(server) => server,
            None => {
                default_block = MlxServerConfig::default();
                &default_block
            }
        };
        let budget = if server.thinking_budget > 0 {
            server.thinking_budget
        } else {
            AUTO_THINKING_BUDGET
        };
        // The request field is a `mlx_vlm.server` feature, and that server
        // rejects it outright alongside speculative decoding.
        let budget_supported = server.is_vlm() && server.draft_model.trim().is_empty();
        Self {
            temperature: if server.temp >= 0.0 {
                f64::from(server.temp)
            } else {
                DEFAULT_TEMPERATURE
            },
            top_p: if server.top_p >= 0.0 {
                f64::from(server.top_p)
            } else {
                DEFAULT_TOP_P
            },
            top_k: if server.top_k >= 0 {
                i64::from(server.top_k)
            } else {
                DEFAULT_TOP_K
            },
            min_p: (server.min_p >= 0.0).then(|| f64::from(server.min_p)),
            max_tokens: if server.max_tokens > 0 {
                server.max_tokens
            } else {
                DEFAULT_MAX_TOKENS.max(budget.saturating_add(ANSWER_HEADROOM_TOKENS))
            },
            thinking_budget: budget_supported.then_some(budget),
        }
    }

    /// Fill `request` where the caller left a field unset.
    pub(crate) fn apply(&self, request: &mut ModelRequest) {
        if request.temperature.is_none() {
            request.temperature = Some(self.temperature);
        }
        if request.top_p.is_none() {
            request.top_p = Some(self.top_p);
        }
        if request.max_tokens.is_none() {
            request.max_tokens = Some(self.max_tokens);
        }

        let mut extras = serde_json::Map::new();
        extras.insert("top_k".into(), json!(self.top_k));
        if let Some(min_p) = self.min_p {
            extras.insert("min_p".into(), json!(min_p));
        }
        if let Some(budget) = self.thinking_budget {
            if thinking_budget_applies(&request.provider_options) {
                extras.insert("thinking_budget".into(), json!(budget));
            }
        }
        if !request.provider_options.is_object() {
            request.provider_options = Value::Object(serde_json::Map::new());
        }
        if let Some(options) = request.provider_options.as_object_mut() {
            for (key, value) in extras {
                options.entry(key).or_insert(value);
            }
        }
    }
}

/// Whether a default thinking budget makes sense next to the options already on
/// the request: not when the turn asked for no thinking (nothing to bound) and
/// not when it asked for `high` effort (the turn-controls contract is that a
/// ceiling would contradict the ask).
fn thinking_budget_applies(options: &Value) -> bool {
    if options.get("enable_thinking").and_then(Value::as_bool) == Some(false) {
        return false;
    }
    options.get("reasoning_effort").and_then(Value::as_str) != Some("high")
}

/// The `[[mlx.server]]` block that serves `model_id`: the block whose `model`
/// slot names it, else the one the endpoint resolver would dial, else the first.
pub(crate) fn server_for_model<'a>(
    config: &'a Config,
    model_id: &str,
) -> Option<&'a MlxServerConfig> {
    let servers = &config.mlx.servers;
    let wanted = model_id.trim();
    servers
        .iter()
        .find(|server| !wanted.is_empty() && server.model.trim() == wanted)
        .or_else(|| servers.iter().find(|server| server.port != 0))
        .or_else(|| servers.first())
}

/// The temperature the turn path should use as its *default* for an `mlx:`
/// model, in place of the global `default_temperature`.
///
/// The turn path fills an unset request temperature with the role default
/// before the request reaches [`MlxTunedModel`], so by then it can no longer
/// tell a caller's value from the filler. Resolving the default here, where the
/// filler is chosen, keeps explicit values (per-turn controls, the `@temp`
/// suffix) winning and lets the block's own `temp` or the Qwen default replace
/// only the generic global.
pub(crate) fn turn_default_temperature(config: &Config, model_id: &str) -> f64 {
    MlxRequestDefaults::for_block(server_for_model(config, model_id)).temperature
}

/// Wrap an `mlx:` chat model with request defaults and text-tool recovery.
pub(crate) fn tune_mlx_chat_model(
    chat: Arc<dyn ChatModel<()>>,
    model_id: &str,
    config: &Config,
) -> Arc<dyn ChatModel<()>> {
    let defaults = MlxRequestDefaults::for_block(server_for_model(config, model_id));
    log::debug!(
        "{LOG} model={model_id} temperature={} top_p={} top_k={} min_p={:?} max_tokens={} thinking_budget={:?}",
        defaults.temperature,
        defaults.top_p,
        defaults.top_k,
        defaults.min_p,
        defaults.max_tokens,
        defaults.thinking_budget
    );
    Arc::new(MlxTunedModel {
        inner: chat,
        defaults,
    })
}

/// A chat model that fills MLX request defaults and recovers text tool calls.
pub(crate) struct MlxTunedModel {
    inner: Arc<dyn ChatModel<()>>,
    defaults: MlxRequestDefaults,
}

/// Advertised tools as owned `(name, parameters)` pairs, so a stream can carry
/// them past the request that supplied them.
#[derive(Clone, Default)]
struct AdvertisedTools(Vec<(String, Value)>);

impl AdvertisedTools {
    fn from_request(request: &ModelRequest) -> Self {
        Self(
            request
                .tools
                .iter()
                .map(|tool| (tool.name.clone(), tool.parameters.clone()))
                .collect(),
        )
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn known(&self) -> KnownTools<'_> {
        self.0
            .iter()
            .map(|(name, schema)| (name.as_str(), schema))
            .collect()
    }
}

/// Convert recovered calls into the response: calls appended, text replaced.
fn apply_recovery(mut response: ModelResponse, recovery: Recovery) -> ModelResponse {
    for (slot, call) in recovery.calls.into_iter().enumerate() {
        response.message.tool_calls.push(ToolCall {
            id: next_synthetic_call_id(slot + 1),
            name: call.name,
            arguments: call.arguments,
            invalid: None,
        });
    }
    response.message.content = replace_text(response.message.content, recovery.text);
    response.finish_reason = Some("tool_calls".to_string());
    response
}

/// Keep every non-text block (reasoning, extensions) in place and substitute the
/// cleaned text at the first text block's position.
fn replace_text(content: Vec<ContentBlock>, cleaned: String) -> Vec<ContentBlock> {
    let mut out = Vec::with_capacity(content.len());
    let mut placed = false;
    for block in content {
        match block {
            ContentBlock::Text(_) => {
                if !placed {
                    if !cleaned.is_empty() {
                        out.push(ContentBlock::Text(cleaned.clone()));
                    }
                    placed = true;
                }
            }
            other => out.push(other),
        }
    }
    if !placed && !cleaned.is_empty() {
        out.push(ContentBlock::Text(cleaned));
    }
    out
}

/// Recover text-form calls from a completed response when it carries none.
fn recover_in_response(response: ModelResponse, tools: &AdvertisedTools) -> ModelResponse {
    if tools.is_empty() || !response.message.tool_calls.is_empty() {
        return response;
    }
    let text = response.text();
    match recover_text_tool_calls(&text, Some(&tools.known())) {
        Some(recovery) => {
            log::info!(
                "{LOG} native response carried no tool_calls; recovered {} call(s) from text",
                recovery.calls.len()
            );
            apply_recovery(response, recovery)
        }
        None => response,
    }
}

/// Holds back streamed text from a bare `<function=` opener to the end of the
/// stream, so a call the model wrote as text is not rendered live and then
/// removed from the final message.
#[derive(Default)]
struct BareFunctionFilter {
    buf: String,
}

impl BareFunctionFilter {
    fn feed(&mut self, fragment: &str) -> String {
        self.buf.push_str(fragment);
        let hold = hold_from_for_bare_function(&self.buf);
        let out = self.buf[..hold].to_string();
        self.buf.drain(..hold);
        out
    }

    fn take_held(&mut self) -> String {
        std::mem::take(&mut self.buf)
    }
}

fn filter_stream(inner: ModelStream, tools: AdvertisedTools) -> ModelStream {
    let mut filter = BareFunctionFilter::default();
    Box::pin(
        inner.flat_map(move |item| futures::stream::iter(filter_item(item, &mut filter, &tools))),
    )
}

fn filter_item(
    item: ModelStreamItem,
    filter: &mut BareFunctionFilter,
    tools: &AdvertisedTools,
) -> VecDeque<ModelStreamItem> {
    let mut out = VecDeque::new();
    match item {
        ModelStreamItem::MessageDelta(mut delta) => {
            if !delta.text.is_empty() {
                delta.text = filter.feed(&delta.text);
            }
            if !(delta.text.is_empty() && delta.reasoning.is_empty() && delta.tool_call.is_none()) {
                out.push_back(ModelStreamItem::MessageDelta(delta));
            }
        }
        ModelStreamItem::Completed(response) => {
            let held = filter.take_held();
            let had_calls = !response.message.tool_calls.is_empty();
            let recovered = recover_in_response(response, tools);
            let recovered_now = !had_calls && !recovered.message.tool_calls.is_empty();
            // Held text that was not a call is real prose: hand it back before
            // the terminal item so delta-rebuilt text matches the response.
            if !held.is_empty() && !recovered_now {
                out.push_back(ModelStreamItem::MessageDelta(MessageDelta::text(held)));
            }
            out.push_back(ModelStreamItem::Completed(recovered));
        }
        other => out.push_back(other),
    }
    out
}

#[async_trait]
impl ChatModel<()> for MlxTunedModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    fn cache_identity(&self) -> Option<String> {
        self.inner.cache_identity()
    }

    async fn invoke(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        self.defaults.apply(&mut request);
        let tools = AdvertisedTools::from_request(&request);
        let response = self.inner.invoke(state, request).await?;
        Ok(recover_in_response(response, &tools))
    }

    async fn stream(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyagents::Result<ModelStream> {
        self.defaults.apply(&mut request);
        let tools = AdvertisedTools::from_request(&request);
        let stream = self.inner.stream(state, request).await?;
        if tools.is_empty() {
            return Ok(stream);
        }
        Ok(filter_stream(stream, tools))
    }
}

#[cfg(test)]
#[path = "mlx_tuning_tests.rs"]
mod tests;
