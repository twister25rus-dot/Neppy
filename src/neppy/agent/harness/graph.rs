//! The **channel/CLI turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this is the harness's top-level
//! (channel/CLI) graph definition, its available tools, and its summarization
//! step — all thin over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`]).
//!
//! **Graph.** A single agent-loop turn driven by the tinyagents harness (the
//! canonical channel/CLI path; the legacy `run_tool_call_loop` is removed),
//! covering the loop's control-flow seams (iteration cap, circuit breakers, stop
//! hooks). When the caller supplies an `on_progress` sender the harness event
//! stream is mirrored onto `AgentProgress` (live tool timeline, streaming text
//! deltas, cost/token footer) via the same
//! `NeppyEventBridge`
//! the chat route uses.
//!
//! **Available tools.** Reuses the bus handler's `Arc`-shared tool sets
//! (`tools_registry: Arc<Vec<Box<dyn Tool>>>` + per-turn `extra_tools`),
//! advertised via `SharedToolAdapter`
//! and filtered by `visible_tool_names`. No early-exit tools on this path.
//!
//! **Summarization.** [`run_channel_turn_via_graph`] resolves the model's
//! effective context window before dispatch so the shared seam runs the
//! context-window summarization step (`tinyagents::summarize`) ahead of the
//! deterministic front-trim.

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::mpsc::Sender;

use crate::neppy::agent::messages::ChatMessage;
use crate::neppy::agent::progress::AgentProgress;
use crate::neppy::agent::tinyagents::run_turn_via_tinyagents_shared;
use crate::neppy::agent::tinyagents::TurnModelSource;
use crate::neppy::config::{MultimodalConfig, MultimodalFileConfig};
use crate::neppy::tools::Tool;

/// Thread mode on the channel/CLI path: a channel turn declares no thread mode,
/// so unless the caller scoped Orchestration mode it gets chat semantics — the
/// multi-agent fleet tools (`web_chat::mode::CHAT_HIDDEN_TOOLS`) are removed
/// from the callable set even when the routed definition (the orchestrator)
/// names them. `None` ("every registered tool") is materialised from the tool
/// sets only when a fleet tool is actually among them, so an unaffected turn
/// keeps its unfiltered shape.
pub(crate) fn channel_turn_allowed_tools(
    allowed: Option<HashSet<String>>,
    tool_sets: &[&[Box<dyn Tool>]],
    declared: Option<crate::neppy::threads::mode::ThreadMode>,
) -> Option<HashSet<String>> {
    use crate::neppy::web_chat::mode::CHAT_HIDDEN_TOOLS;
    if declared == Some(crate::neppy::threads::mode::ThreadMode::Orchestration) {
        return allowed;
    }
    let is_fleet = |name: &str| CHAT_HIDDEN_TOOLS.contains(&name);
    match allowed {
        Some(mut set) => {
            let before = set.len();
            set.retain(|name| !is_fleet(name));
            if set.len() != before {
                tracing::debug!(
                    hidden = before - set.len(),
                    "[channel:graph] no Orchestration mode declared: hid fleet tools"
                );
            }
            Some(set)
        }
        None => {
            let names: Vec<&str> = tool_sets
                .iter()
                .flat_map(|set| set.iter())
                .map(|t| t.name())
                .collect();
            if !names.iter().any(|n| is_fleet(n)) {
                return None;
            }
            tracing::debug!(
                "[channel:graph] no Orchestration mode declared: hid fleet tools from the \
                 unfiltered set"
            );
            Some(
                names
                    .into_iter()
                    .filter(|n| !is_fleet(n))
                    .map(str::to_string)
                    .collect(),
            )
        }
    }
}

/// Drive a channel/CLI turn on the graph engine. Returns the final assistant
/// text. When `on_progress` is `Some`, the run streams and mirrors progress
/// onto `AgentProgress`; pass `None` for a fire-and-forget final-text turn.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_channel_turn_via_graph(
    source: TurnModelSource,
    history: &mut Vec<ChatMessage>,
    tools_registry: Arc<Vec<Box<dyn Tool>>>,
    extra_tools: Vec<Box<dyn Tool>>,
    visible_tool_names: Option<&HashSet<String>>,
    model: &str,
    temperature: f64,
    max_iterations: usize,
    multimodal: MultimodalConfig,
    multimodal_files: MultimodalFileConfig,
    on_progress: Option<Sender<AgentProgress>>,
) -> Result<String> {
    let extra_arc = Arc::new(extra_tools);

    // The callable set is the visibility whitelist. The runner advertises each via
    // its own `spec()`, deduped by name (extras shadow the registry).
    // Fail-closed allowlist plumbing (issue #4452): the shared seam takes an
    // `Option<HashSet<String>>` where `None` = no filter (all visible tools) and
    // `Some(set)` = exactly those tools. The channel/CLI path's historical
    // convention is "no filter / empty set = every visible tool", so map both a
    // missing filter and an empty set to `None`; only a populated set is treated
    // as an explicit whitelist.
    let allowed: Option<HashSet<String>> = match visible_tool_names {
        Some(set) if !set.is_empty() => Some(set.clone()),
        _ => None,
    };
    let allowed = channel_turn_allowed_tools(
        allowed,
        &[extra_arc.as_slice(), tools_registry.as_slice()],
        crate::neppy::threads::mode::current_turn_mode(),
    );

    // Resolve the model's effective context window (async provider probe) so the
    // harness can run the context-window summarization step (issue #4249) on
    // channel/CLI turns too — long-running channel threads otherwise grew
    // unbounded until the cap error — then build the turn's crate `ChatModel` set.
    // The `Provider` is confined to the seam `TurnModelSource` (issue #4249,
    // Phase 3 / Motion A): the harness graph names crate model types only, and
    // reads native-tool / vision capability + telemetry id off the built bundle.
    let context_window = source.effective_context_window(model).await;
    let turn_models = source.build(model, temperature, context_window)?;

    // Native-tool support drives the durable history-suffix dispatcher (native
    // envelope vs prompt-guided text) at the end of this turn; capture it before
    // `turn_models` is moved into the runner.
    let native_tools = turn_models.native_tools();
    let provider_id = turn_models.provider_id().to_string();

    // Multimodal prep (parity with the chat route's
    // `run_turn_via_tinyagents_session`, issue #4249): rehydrate image
    // placeholders for vision-capable models, then expand `[IMAGE:…]` /
    // `[FILE:…]` markers into provider-ready content before dispatch. The
    // expanded copy is provider-only — it is sent to the model but never
    // persisted back into the channel `history` (see the reconstruction below).
    let mut prepared = history.clone();
    if turn_models.supports_vision()
        && crate::neppy::agent::multimodal::has_image_placeholders(&prepared)
    {
        prepared = crate::neppy::agent::multimodal::rehydrate_image_placeholders(&prepared);
    }
    let prepared = crate::neppy::agent::multimodal::prepare_messages_for_provider(
        &prepared,
        &multimodal,
        &multimodal_files,
    )
    .await
    .map(|prepared| prepared.messages)
    .unwrap_or(prepared);

    tracing::info!(
        model,
        max_iterations,
        observed = on_progress.is_some(),
        context_window,
        "[channel:graph] routing channel turn through tinyagents harness"
    );
    let outcome = run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        model,
        prepared,
        vec![extra_arc, tools_registry],
        allowed,
        max_iterations,
        // Mirror the harness event stream onto AgentProgress when the caller
        // (e.g. channel dispatch) supplied a progress sink.
        on_progress,
        // Top-level (parent) turn — no child-progress attribution.
        None,
        // Resolved above — drives the context-window summarization step.
        context_window,
        // No mid-flight steering on the channel path.
        None,
        // No early-exit pause on the channel path.
        &[],
        // Channels surface the cap as an error (legacy `ErrorCheckpoint`), so no
        // graceful cap pause/summary here.
        false,
        // Bound the model's per-call output (legacy parity — channel turns ran at
        // the standard per-turn budget).
        Some(crate::neppy::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS),
        // Context middlewares: cache-align + default tool-result byte cap (the
        // channel path has no session `ContextManager` to source config from).
        crate::neppy::agent::tinyagents::TurnContextMiddleware::defaults(),
        // Channel/CLI path carries its own gating; no session `.tool_policy()`.
        None,
        // Channel turns do not yet carry SDK workspace descriptors.
        None,
        // Interactive channel/CLI turn — never serve a cached model response.
        false,
        // #4457 (defect C): the channel/CLI path has no post-run wrap-up and does
        // NOT emit `TurnCompleted` itself, so let the seam emit the single
        // terminal event (legacy-engine parity).
        false,
    )
    .await?;
    // Append only this turn's typed suffix (assistant tool-calls + tool results +
    // final assistant), serialized with the matching dispatcher so a native tool
    // round persists as the `{content, tool_calls}` / `{tool_call_id, content}`
    // envelope (re-parsed by `convert::chat_message_to_message` next turn) rather
    // than an assistant with no `tool_calls` followed by an orphan `tool` row.
    // Using `outcome.conversation` (the typed messages-since-last-user) avoids
    // indexing into a post-trim `outcome.history` with the pre-trim `prior_len`,
    // which could drop current-turn messages when compaction reshaped the run.
    use crate::neppy::agent::dispatcher::ToolDispatcher;
    let suffix = if native_tools {
        crate::neppy::agent::dispatcher::NativeToolDispatcher
            .to_provider_messages(&outcome.conversation)
    } else {
        // History serialization is format-independent for prompt-guided providers
        // (tool calls already ride the visible assistant text); the XML dispatcher
        // renders the flat `[Tool results]` shape.
        crate::neppy::agent::dispatcher::XmlToolDispatcher
            .to_provider_messages(&outcome.conversation)
    };
    history.extend(suffix);
    Ok(outcome.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::tools::ToolResult;
    use async_trait::async_trait;
    use tinyagents::harness::message::AssistantMessage;
    use tinyagents::harness::model::{ChatModel, ModelProfile, ModelResponse};
    use tinyagents::harness::testkit::ScriptedModel;
    use tinyagents::harness::tool::ToolCall;

    struct PingTool;
    #[async_trait]
    impl Tool for PingTool {
        fn name(&self) -> &str {
            "ping"
        }
        fn description(&self) -> &str {
            "ping"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("pong"))
        }
    }

    struct NamedTool(&'static str);
    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "named"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    /// W1: a channel/CLI turn declares no thread mode, so the orchestrator's
    /// fleet tools are not callable — whether the routed definition named them
    /// (explicit allowlist) or the turn is unfiltered. Orchestration mode keeps
    /// them; blocking delegation stays either way.
    #[test]
    fn channel_turn_without_orchestration_mode_hides_the_fleet_tools() {
        use crate::neppy::threads::mode::ThreadMode;
        let registry: Vec<Box<dyn Tool>> = vec![
            Box::new(NamedTool("spawn_subagent")),
            Box::new(NamedTool("spawn_parallel_agents")),
            Box::new(NamedTool("wait_subagent")),
            Box::new(NamedTool("steer_subagent")),
            Box::new(NamedTool("close_subagent")),
            Box::new(NamedTool("delegate_researcher")),
        ];
        let sets: [&[Box<dyn Tool>]; 1] = [registry.as_slice()];
        let named: HashSet<String> = registry.iter().map(|t| t.name().to_string()).collect();
        let fleet = [
            "spawn_parallel_agents",
            "wait_subagent",
            "steer_subagent",
            "close_subagent",
        ];

        for declared in [None, Some(ThreadMode::Chat)] {
            for allowed in [Some(named.clone()), None] {
                let got = channel_turn_allowed_tools(allowed, &sets, declared)
                    .expect("fleet present, so the set is materialised");
                for f in fleet {
                    assert!(!got.contains(f), "{declared:?}: {f} must be hidden");
                }
                assert!(got.contains("spawn_subagent"));
                assert!(got.contains("delegate_researcher"));
            }
        }
        // Orchestration mode keeps the supervisor belt untouched.
        assert_eq!(
            channel_turn_allowed_tools(Some(named.clone()), &sets, Some(ThreadMode::Orchestration)),
            Some(named)
        );
        assert_eq!(
            channel_turn_allowed_tools(None, &sets, Some(ThreadMode::Orchestration)),
            None
        );
        // An unfiltered turn with no fleet tool keeps its unfiltered shape.
        let plain: Vec<Box<dyn Tool>> = vec![Box::new(PingTool)];
        assert_eq!(
            channel_turn_allowed_tools(None, &[plain.as_slice()], None),
            None
        );
    }

    #[tokio::test]
    async fn channel_turn_runs_through_the_graph() {
        let registry: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(PingTool)]);
        let mut history = vec![ChatMessage::user("ping please")];
        let scripted: Arc<dyn ChatModel<()>> = Arc::new(ScriptedModel::new(vec![
            ModelResponse {
                message: AssistantMessage {
                    id: None,
                    content: Vec::new(),
                    tool_calls: vec![ToolCall::new("p", "ping", serde_json::json!({}))],
                    usage: None,
                },
                usage: None,
                finish_reason: Some("tool_calls".to_string()),
                raw: None,
                resolved_model: None,
                continue_turn: None,
                served_from_cache: false,
            },
            ModelResponse::assistant("channel done"),
        ]));
        let mut profile = ModelProfile::default();
        profile.tool_calling = true;
        profile.parallel_tool_calls = true;
        let text = run_channel_turn_via_graph(
            TurnModelSource::from_model_with_profile(scripted, profile),
            &mut history,
            registry,
            vec![],
            None,
            "mock-model",
            0.0,
            10,
            MultimodalConfig::default(),
            MultimodalFileConfig::default(),
            None,
        )
        .await
        .expect("channel graph turn runs");
        assert_eq!(text, "channel done");
        assert!(history.iter().any(|m| m.content.contains("pong")));
    }
}
