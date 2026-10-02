//! M2: a sub-agent stopped by its wall-clock deadline returns the work it
//! completed as an `Incomplete` checkpoint instead of an error that loses it.

use super::*;
use crate::neppy::tools::ToolResult;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tinyagents::harness::message::AssistantMessage;
use tinyagents::harness::model::{ChatModel, ModelProfile, ModelRequest, ModelResponse};
use tinyagents::harness::tool::ToolCall;

fn profile() -> &'static ModelProfile {
    static PROFILE: std::sync::LazyLock<ModelProfile> = std::sync::LazyLock::new(|| ModelProfile {
        provider: Some("subagent-timeout-test".to_string()),
        tool_calling: true,
        parallel_tool_calls: true,
        streaming: true,
        ..ModelProfile::default()
    });
    &PROFILE
}

/// First call asks for `echo`; every later call hangs far past any deadline.
struct EchoThenHang {
    calls: AtomicUsize,
}

#[async_trait]
impl ChatModel<()> for EchoThenHang {
    fn profile(&self) -> Option<&ModelProfile> {
        Some(profile())
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(ModelResponse {
                message: AssistantMessage {
                    id: None,
                    content: Vec::new(),
                    tool_calls: vec![ToolCall::new(
                        "1",
                        "echo",
                        serde_json::json!({ "msg": "partial-progress-marker" }),
                    )],
                    usage: None,
                },
                usage: None,
                finish_reason: Some("tool_calls".to_string()),
                raw: None,
                resolved_model: None,
                continue_turn: None,
                served_from_cache: false,
            });
        }
        tokio::time::sleep(Duration::from_secs(120)).await;
        Ok(ModelResponse::assistant("too late"))
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
        Ok(ToolResult::success(format!("echoed:{m}")))
    }
}

#[tokio::test]
async fn timed_out_subagent_returns_incomplete_checkpoint_with_partial_work() {
    let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(EchoTool)]);
    let mut allowed = HashSet::new();
    allowed.insert("echo".to_string());
    let mut history = vec![ChatMessage::user("echo something, then keep going")];
    let workspace = tempfile::TempDir::new().unwrap();

    // The enclosing (parent) run has ~2 s left; the nested child inherits that
    // minus grace and must time out on its own, well before the hung call.
    let started = Instant::now();
    let result = crate::neppy::agent::tinyagents::with_turn_deadline(
        started + Duration::from_secs(2),
        run_subagent_via_graph(
            crate::neppy::agent::tinyagents::TurnModelSource::from_model(Arc::new(EchoThenHang {
                calls: AtomicUsize::new(0),
            })),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            10,
            None,
            None,
            "researcher",
            "task-timeout",
            false,
            None,
            workspace.path().to_path_buf(),
            None,
            1024,
            false,
            "root-session__timeout",
            "mock-channel",
            None,
            AgentTokenjuiceCompression::Off,
            None,
        ),
    )
    .await;
    let elapsed = started.elapsed();

    let (output, _iterations, _usage, early_exit, hit_cap, halt) =
        result.expect("a wall-clock timeout must not surface as an error");
    assert!(
        elapsed < Duration::from_secs(10),
        "the child must stop at its own deadline, took {elapsed:?}"
    );
    assert_eq!(halt.as_deref(), Some("timed out"));
    assert!(!hit_cap);
    assert!(early_exit.is_none());
    assert!(output.contains("ran out of time"), "{output}");
    assert!(
        output.contains("echoed:partial-progress-marker"),
        "the checkpoint must carry the completed tool round, got {output:?}"
    );
    assert!(
        history
            .last()
            .is_some_and(|m| m.role == "assistant" && m.content == output),
        "the checkpoint closes the returned history"
    );
}

#[test]
fn nested_budget_leaves_the_parent_a_grace_window() {
    use crate::neppy::agent::tinyagents::nested_run_budget_ms;
    // 10 min left → 60 s grace (10% capped at 60 s).
    assert_eq!(nested_run_budget_ms(Duration::from_secs(600)), 540_000);
    // 30 s left → 3 s grace.
    assert_eq!(nested_run_budget_ms(Duration::from_secs(30)), 27_000);
    // Under the 2 s floor → half of what is left; never zero.
    assert_eq!(nested_run_budget_ms(Duration::from_secs(2)), 1_000);
    assert_eq!(nested_run_budget_ms(Duration::ZERO), 1);
}
