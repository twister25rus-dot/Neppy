//! `ImageAwareMessageTrimMiddleware` when the system prompt alone exceeds the
//! budget: history must degrade to a floor, not to the single newest message.

use super::*;
use tinyagents::harness::context::{RunConfig, RunContext};

fn ctx() -> RunContext<()> {
    RunContext::new(RunConfig::new("trim-test"), ())
}

async fn trim(window: u64, messages: Vec<TaMessage>) -> Vec<TaMessage> {
    let mw = ImageAwareMessageTrimMiddleware::for_context_window(window);
    let mut request = ModelRequest {
        messages,
        ..Default::default()
    };
    mw.before_model(&mut ctx(), &(), &mut request)
        .await
        .unwrap();
    request.messages
}

fn texts(messages: &[TaMessage]) -> Vec<String> {
    messages.iter().map(|m| m.text()).collect()
}

#[tokio::test]
async fn small_window_still_evicts_history_when_the_prompt_fits() {
    // Budget (window 1000 -> ~900): system 10 tokens, history well over budget.
    let mut messages = vec![TaMessage::system("sys")];
    for i in 0..20 {
        messages.push(TaMessage::user(format!("u{i} {}", "x".repeat(400))));
        messages.push(TaMessage::assistant(format!("a{i} {}", "y".repeat(400))));
    }
    let kept = trim(1000, messages).await;
    assert!(
        kept.len() < 41,
        "history is trimmed as before: {}",
        kept.len()
    );
    assert!(matches!(kept[0], TaMessage::System(_)));
    assert!(
        !texts(&kept).iter().any(|t| t.starts_with("u0 ")),
        "oldest message evicted in the normal regime"
    );
}

#[tokio::test]
async fn oversized_system_prompt_keeps_the_task_and_recent_history() {
    // ~35k-token system prompt against a 4096 window (budget 3584): the shape
    // that made a local model forget the task and repeat one tool call.
    let mut messages = vec![
        TaMessage::system("S".repeat(140_000)),
        TaMessage::user("the task: fix the three UI bugs"),
    ];
    for i in 0..3 {
        messages.push(TaMessage::assistant(format!("calling tool {i}")));
        messages.push(TaMessage::tool(format!("c{i}"), format!("result {i}")));
    }
    let kept = trim(4096, messages).await;
    let t = texts(&kept);
    assert!(
        t.iter()
            .any(|s| s.contains("the task: fix the three UI bugs")),
        "the user's request survives: {t:?}"
    );
    assert!(
        t.iter().any(|s| s == "result 2") && t.iter().any(|s| s == "calling tool 0"),
        "recent exchanges survive instead of only the newest message: {t:?}"
    );
}

#[tokio::test]
async fn pinned_task_is_not_followed_by_an_orphaned_tool_result() {
    // Enough history that eviction must drop the oldest assistant(tool_calls)
    // while keeping its tool answer: that answer must be snapped away.
    let mut messages = vec![
        TaMessage::system("S".repeat(140_000)),
        TaMessage::user("task"),
        TaMessage::assistant("first call"),
        TaMessage::tool("c0", "z".repeat(60_000)),
    ];
    for i in 1..4 {
        messages.push(TaMessage::assistant(format!("call {i}")));
        messages.push(TaMessage::tool(format!("c{i}"), "w".repeat(20_000)));
    }
    let kept = trim(4096, messages).await;
    let first_after_task = kept
        .iter()
        .position(|m| matches!(m, TaMessage::User(_)))
        .map(|i| &kept[i + 1]);
    if let Some(next) = first_after_task {
        assert!(
            !matches!(next, TaMessage::Tool(_)),
            "no orphaned tool result right after the pinned task"
        );
    }
    assert!(kept.iter().any(|m| m.text() == "task"));
}
