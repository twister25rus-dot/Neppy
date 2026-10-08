use serde_json::json;

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::pass_gate::is_real_check;
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::agent::debug_mode::turn::{begin, resolve_root, with_turn};

async fn in_turn<F: std::future::Future>(
    repo: &tempfile::TempDir,
    ws: &tempfile::TempDir,
    fut: F,
) -> (crate::neppy::agent::debug_mode::turn::DebugTurn, F::Output) {
    let root = resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let turn = begin(ws.path(), root, "check test").await;
    let out = with_turn(turn.clone(), fut).await;
    (turn, out)
}

#[tokio::test]
async fn refuses_outside_a_debug_turn() {
    let r = DebugRunCheckTool
        .execute(json!({"command": ["git", "status"]}))
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.text().contains("Debug-mode turn"), "{}", r.text());
}

#[tokio::test]
async fn persists_a_real_validation_record_on_the_current_task() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let (turn, r) = in_turn(&repo, &ws, async {
        DebugRunCheckTool
            .execute(json!({"command": ["git", "rev-parse", "HEAD"], "timeout_secs": 30}))
            .await
            .unwrap()
    })
    .await;
    assert!(!r.is_error, "{}", r.text());
    let out: serde_json::Value = serde_json::from_str(&r.output()).unwrap();
    assert_eq!(out["passed"], json!(true));
    assert_eq!(out["exit_code"], json!(0));
    assert_eq!(out["output_tail"].as_str().unwrap().trim().len(), 40);
    assert!(out["duration_ms"].is_u64());

    let ctx = DebugCtx::new(ws.path());
    let task = ctx
        .store
        .task_get(turn.task_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(task.validation.len(), 1);
    let v = &task.validation[0];
    assert_eq!(v.command, ["git", "rev-parse", "HEAD"]);
    assert_eq!(v.exit_code, Some(0));
    assert!(v.passed && !v.timed_out);
    assert!(chrono::DateTime::parse_from_rfc3339(&v.at).is_ok());
    assert!(is_real_check(v));
}

#[tokio::test]
async fn a_failing_check_is_recorded_as_failed() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let (turn, r) = in_turn(&repo, &ws, async {
        DebugRunCheckTool
            .execute(json!({"command": ["git", "rev-parse", "--verify", "nope"]}))
            .await
            .unwrap()
    })
    .await;
    assert!(!r.is_error, "a failing check is a result, not a tool error");
    let out: serde_json::Value = serde_json::from_str(&r.output()).unwrap();
    assert_eq!(out["passed"], json!(false));
    let task = turn
        .ctx
        .store
        .task_get(turn.task_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(task.validation.len(), 1);
    assert!(!task.validation[0].passed && task.validation[0].exit_code != Some(0));
}

#[tokio::test]
async fn only_allowed_commands_run_and_bad_input_is_rejected() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let (turn, rs) = in_turn(&repo, &ws, async {
        let mut v = Vec::new();
        for args in [
            json!({"command": ["sh", "-c", "touch pwned"]}),
            json!({"command": ["git", "reset", "--hard"]}),
            json!({"check_id": "no-such-check"}),
            json!({}),
            json!({"check_id": "x", "command": ["git", "status"]}),
            json!({"command": "git status"}),
            json!({"bogus": 1}),
        ] {
            v.push(DebugRunCheckTool.execute(args).await.unwrap());
        }
        v
    })
    .await;
    for r in &rs {
        assert!(r.is_error, "{}", r.text());
    }
    assert!(!repo.path().join("pwned").exists());
    let task = turn
        .ctx
        .store
        .task_get(turn.task_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert!(task.validation.is_empty(), "rejected runs record nothing");
}
