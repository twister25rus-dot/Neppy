use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tinyagents::harness::context::RunConfig;
use tinyagents::harness::middleware::{BoxToolFuture, MiddlewareStack, ToolBaseCall};
use tinyagents::harness::tool::ToolCall;

// ── pure state machine ────────────────────────────────────────────────────

fn run(state: &mut LoopGuardState, tool: &str, args: Value, result: &str) -> Verdict {
    let verdict = state.check(tool, &args);
    if verdict == Verdict::Proceed {
        state.record_success(tool, &args, result);
    }
    verdict
}

#[test]
fn third_identical_call_with_identical_result_is_nudged() {
    let mut s = LoopGuardState::default();
    assert_eq!(
        run(&mut s, "read_state", json!({"a": 1}), "same"),
        Verdict::Proceed
    );
    assert_eq!(
        run(&mut s, "read_state", json!({"a": 1}), "same"),
        Verdict::Proceed
    );
    match run(&mut s, "read_state", json!({"a": 1}), "same") {
        Verdict::Nudge(text) => {
            assert!(text.contains("NOT run"), "{text}");
            assert!(text.contains("`read_state`"), "{text}");
            assert!(text.contains("2 times"), "{text}");
            assert!(text.contains("same"), "quotes the repeated result: {text}");
            assert!(text.contains("Do not repeat it"), "{text}");
        }
        other => panic!("expected a nudge, got {other:?}"),
    }
}

#[test]
fn argument_key_order_does_not_hide_a_repeat() {
    let mut s = LoopGuardState::default();
    run(&mut s, "t", json!({"a": 1, "b": {"x": 1, "y": 2}}), "r");
    run(&mut s, "t", json!({"b": {"y": 2, "x": 1}, "a": 1}), "r");
    assert!(matches!(
        run(&mut s, "t", json!({"a": 1, "b": {"x": 1, "y": 2}}), "r"),
        Verdict::Nudge(_)
    ));
}

#[test]
fn different_arguments_never_trip_the_guard() {
    let mut s = LoopGuardState::default();
    for i in 0..10 {
        assert_eq!(
            run(&mut s, "grep", json!({"pattern": format!("p{i}")}), "same"),
            Verdict::Proceed,
            "call {i} has fresh arguments"
        );
    }
}

#[test]
fn identical_calls_with_changing_results_are_progress_not_a_loop() {
    let mut s = LoopGuardState::default();
    for i in 0..6 {
        assert_eq!(
            run(&mut s, "tail_log", json!({}), &format!("line {i}")),
            Verdict::Proceed
        );
    }
}

#[test]
fn a_different_call_in_between_breaks_the_streak() {
    let mut s = LoopGuardState::default();
    run(&mut s, "a", json!({}), "r");
    run(&mut s, "a", json!({}), "r");
    assert_eq!(run(&mut s, "b", json!({}), "r2"), Verdict::Proceed);
    assert_eq!(run(&mut s, "a", json!({}), "r"), Verdict::Proceed);
}

#[test]
fn persistent_loop_after_the_nudge_ends_the_turn_gracefully() {
    let mut s = LoopGuardState::default();
    run(&mut s, "read_state", json!({}), "same");
    run(&mut s, "read_state", json!({}), "same");
    assert!(matches!(
        run(&mut s, "read_state", json!({}), "same"),
        Verdict::Nudge(_)
    ));
    match run(&mut s, "read_state", json!({}), "same") {
        Verdict::Halt { tool_text, summary } => {
            assert!(summary.starts_with("Stopping:"), "{summary}");
            assert!(summary.contains("`read_state`"), "{summary}");
            assert!(summary.contains("same"), "{summary}");
            assert!(
                summary.contains("Tell me"),
                "invites the user to steer: {summary}"
            );
            assert!(tool_text.contains("turn is ending"), "{tool_text}");
        }
        other => panic!("expected a halt, got {other:?}"),
    }
}

#[test]
fn model_that_changes_course_after_the_nudge_keeps_going() {
    let mut s = LoopGuardState::default();
    run(&mut s, "a", json!({}), "r");
    run(&mut s, "a", json!({}), "r");
    assert!(matches!(
        run(&mut s, "a", json!({}), "r"),
        Verdict::Nudge(_)
    ));
    assert_eq!(run(&mut s, "b", json!({}), "other"), Verdict::Proceed);
    assert_eq!(run(&mut s, "c", json!({}), "more"), Verdict::Proceed);
}

#[test]
fn abab_cycle_with_identical_results_is_caught() {
    let mut s = LoopGuardState::default();
    assert_eq!(run(&mut s, "a", json!({}), "ra"), Verdict::Proceed);
    assert_eq!(run(&mut s, "b", json!({}), "rb"), Verdict::Proceed);
    assert_eq!(run(&mut s, "a", json!({}), "ra"), Verdict::Proceed);
    assert_eq!(run(&mut s, "b", json!({}), "rb"), Verdict::Proceed);
    match run(&mut s, "a", json!({}), "ra") {
        Verdict::Nudge(text) => {
            assert!(text.contains("cycling"), "{text}");
            assert!(text.contains("`a`") && text.contains("`b`"), "{text}");
        }
        other => panic!("expected a cycle nudge, got {other:?}"),
    }
}

#[test]
fn three_call_cycle_is_caught() {
    let mut s = LoopGuardState::default();
    for _ in 0..2 {
        for t in ["a", "b", "c"] {
            assert_eq!(run(&mut s, t, json!({}), t), Verdict::Proceed);
        }
    }
    assert!(matches!(
        run(&mut s, "a", json!({}), "a"),
        Verdict::Nudge(_)
    ));
}

#[test]
fn cycle_whose_results_change_is_not_a_loop() {
    let mut s = LoopGuardState::default();
    for i in 0..4 {
        let t = if i % 2 == 0 { "a" } else { "b" };
        assert_eq!(
            run(&mut s, t, json!({}), &format!("r{i}")),
            Verdict::Proceed
        );
    }
    assert_eq!(run(&mut s, "a", json!({}), "r4"), Verdict::Proceed);
}

#[test]
fn polling_tools_are_exempt() {
    let mut s = LoopGuardState::default();
    for _ in 0..6 {
        assert_eq!(
            run(&mut s, "wait_subagent", json!({"task_id": "t"}), "running"),
            Verdict::Proceed
        );
    }
}

#[test]
fn an_error_result_resets_the_history() {
    let mut s = LoopGuardState::default();
    run(&mut s, "a", json!({}), "r");
    run(&mut s, "a", json!({}), "r");
    s.record_failure();
    assert_eq!(run(&mut s, "a", json!({}), "r"), Verdict::Proceed);
}

#[test]
fn total_trips_bound_a_model_that_alternates_loops() {
    let mut s = LoopGuardState::default();
    let mut halted = false;
    for round in 0..6 {
        let t = format!("tool{round}");
        run(&mut s, &t, json!({}), "r");
        run(&mut s, &t, json!({}), "r");
        match run(&mut s, &t, json!({}), "r") {
            Verdict::Halt { .. } => {
                halted = true;
                break;
            }
            Verdict::Nudge(_) => {}
            Verdict::Proceed => panic!("round {round} should trip"),
        }
    }
    assert!(halted, "four trips in one turn end it");
}

// ── through the real middleware stack ─────────────────────────────────────

struct CountingTool {
    runs: Arc<AtomicUsize>,
    output: &'static str,
}

impl ToolBaseCall<(), ()> for CountingTool {
    fn call<'a>(
        &'a self,
        _ctx: &'a mut RunContext<()>,
        _state: &'a (),
        call: ToolCall,
    ) -> BoxToolFuture<'a> {
        Box::pin(async move {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(TaToolResult {
                call_id: call.id,
                name: call.name,
                content: self.output.to_string(),
                raw: None,
                error: None,
                elapsed_ms: 0,
            })
        })
    }
}

fn drain_pauses(handle: &SteeringHandle) -> usize {
    handle
        .drain()
        .into_iter()
        .filter(|c| matches!(c, SteeringCommand::Pause))
        .count()
}

#[tokio::test]
async fn middleware_skips_the_third_identical_call_and_pauses_on_the_fourth() {
    let handle = SteeringHandle::allow_all();
    let slot: HaltSummarySlot = Arc::new(Mutex::new(None));
    let mut stack: MiddlewareStack<(), ()> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(LoopGuardMiddleware::new(
        Some(handle.clone()),
        slot.clone(),
    )));
    let runs = Arc::new(AtomicUsize::new(0));
    let base = CountingTool {
        runs: runs.clone(),
        output: "git unavailable",
    };
    let mut ctx = RunContext::new(RunConfig::new("loop-guard-test"), ());
    let mut results = Vec::new();
    for i in 0..4 {
        let call = ToolCall::new(format!("c{i}"), "read_workspace_state", json!({}));
        let MiddlewareToolOutcome::Result(res) = stack
            .run_wrapped_tool(&mut ctx, &(), call, &base)
            .await
            .unwrap()
        else {
            panic!("unexpected outcome variant");
        };
        results.push(res);
    }

    assert!(results[0].error.is_none() && results[1].error.is_none());
    assert_eq!(runs.load(Ordering::SeqCst), 2, "calls 3 and 4 never ran");
    assert!(results[2].error.is_some());
    assert!(
        results[2].content.contains("NOT run"),
        "{}",
        results[2].content
    );
    assert!(results[3].content.contains("turn is ending"));
    assert_eq!(drain_pauses(&handle), 1, "pause only on the persisted loop");
    let summary = slot.lock().unwrap().clone().expect("halt summary latched");
    assert!(summary.starts_with("Stopping:"), "{summary}");
}

#[tokio::test]
async fn middleware_leaves_varied_calls_alone() {
    let handle = SteeringHandle::allow_all();
    let slot: HaltSummarySlot = Arc::new(Mutex::new(None));
    let mut stack: MiddlewareStack<(), ()> = MiddlewareStack::new();
    stack.push_tool_middleware(Arc::new(LoopGuardMiddleware::new(
        Some(handle.clone()),
        slot.clone(),
    )));
    let runs = Arc::new(AtomicUsize::new(0));
    let base = CountingTool {
        runs: runs.clone(),
        output: "same",
    };
    let mut ctx = RunContext::new(RunConfig::new("loop-guard-test"), ());
    for i in 0..8 {
        let call = ToolCall::new(format!("c{i}"), "grep", json!({"pattern": i}));
        let MiddlewareToolOutcome::Result(res) = stack
            .run_wrapped_tool(&mut ctx, &(), call, &base)
            .await
            .unwrap()
        else {
            panic!("unexpected outcome variant");
        };
        assert!(res.error.is_none());
    }
    assert_eq!(runs.load(Ordering::SeqCst), 8);
    assert_eq!(drain_pauses(&handle), 0);
    assert!(slot.lock().unwrap().is_none());
}
