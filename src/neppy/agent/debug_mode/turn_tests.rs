use std::path::Path;

use super::*;
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::agent::debug_mode::types::TaskRecord;
use crate::neppy::agent::turn_workspace;
use crate::neppy::config::schema::debug_mode::DebugModeConfig;

async fn root_of(d: &tempfile::TempDir) -> PathBuf {
    resolve_root(Some(d.path().to_str().unwrap()))
        .await
        .unwrap()
}

async fn only_task(ws: &Path) -> TaskRecord {
    let ctx = DebugCtx::new(ws);
    let tasks = ops::task_list(&ctx, None).await.unwrap().value;
    assert_eq!(tasks.len(), 1, "exactly one task per turn");
    tasks.into_iter().next().unwrap()
}

#[tokio::test]
async fn debug_turn_scopes_the_project_root_around_the_whole_turn() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = root_of(&repo).await;
    assert!(turn_workspace::current().is_none(), "none before the turn");
    assert!(current().is_none());

    let seen = run_in_root(ws.path(), root.clone(), "do a thing", async {
        Ok::<_, String>((turn_workspace::current(), current().map(|t| t.root)))
    })
    .await
    .unwrap();
    assert_eq!(seen.0, Some(root.clone()));
    assert_eq!(seen.1, Some(root));
    assert!(turn_workspace::current().is_none(), "scope must not leak");
    assert!(current().is_none());
}

#[tokio::test]
async fn a_non_debug_turn_sees_no_scope() {
    // Nothing but `run_in_root` establishes the scope; a plain turn has none.
    let seen = async { (turn_workspace::current(), current().is_some()) }.await;
    assert_eq!(seen, (None, false));
}

#[tokio::test]
async fn unchanged_turn_is_recorded_as_pass_with_a_pre_turn_checkpoint() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let long = "x".repeat(900);
    run_in_root(ws.path(), root_of(&repo).await, &long, async {
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    let t = only_task(ws.path()).await;
    assert_eq!(t.status, TaskStatus::Pass);
    assert!(t.files_changed.is_empty());
    assert_eq!(t.request.chars().count(), REQUEST_CLIP, "request truncated");
    let cp = t.checkpoint_id.expect("pre-turn checkpoint recorded");
    let cp = ops::checkpoint_get(&DebugCtx::new(ws.path()), &cp)
        .await
        .unwrap()
        .value;
    assert!(cp.description.starts_with("before: x"));
    assert_eq!(cp.task_id.as_deref(), Some(t.id.as_str()));
}

#[tokio::test]
async fn changed_turn_without_a_report_is_partial_and_lists_files() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let path = repo.path().to_path_buf();
    run_in_root(ws.path(), root_of(&repo).await, "edit", async move {
        std::fs::write(path.join("a.txt"), "two\n").unwrap();
        std::fs::write(path.join("new.txt"), "n\n").unwrap();
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    let t = only_task(ws.path()).await;
    assert_eq!(t.status, TaskStatus::Partial);
    assert_eq!(t.files_changed, ["a.txt", "new.txt"]);
}

#[tokio::test]
async fn the_agents_reported_status_is_kept() {
    use crate::neppy::tools::traits::Tool;
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let path = repo.path().to_path_buf();
    run_in_root(ws.path(), root_of(&repo).await, "edit", async move {
        std::fs::write(path.join("a.txt"), "two\n").unwrap();
        let r = super::super::tools::DebugReportTool
            .execute(serde_json::json!({
                "status": "pass", "summary": "done",
                "validation": [{"check": "cargo check", "passed": true}]
            }))
            .await
            .unwrap();
        assert!(!r.is_error, "{}", r.text());
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    let t = only_task(ws.path()).await;
    assert_eq!(t.status, TaskStatus::Pass);
    assert_eq!(t.summary.as_deref(), Some("done"));
    assert_eq!(t.files_changed, ["a.txt"]);
    assert_eq!(t.validation.len(), 1);
}

#[tokio::test]
async fn errored_turn_is_failed_even_after_a_pass_report_and_the_error_propagates() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let err = run_in_root(ws.path(), root_of(&repo).await, "boom", async {
        Err::<(), _>("provider exploded".to_string())
    })
    .await
    .unwrap_err();
    assert_eq!(err, "provider exploded");
    let t = only_task(ws.path()).await;
    assert_eq!(t.status, TaskStatus::Failed);
    assert!(t.summary.unwrap().contains("provider exploded"));
}

#[tokio::test]
async fn cancelled_turn_is_marked_failed() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = root_of(&repo).await;
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let turn = run_in_root(ws.path(), root, "slow", async move {
        let _ = started_tx.send(());
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        Ok::<_, String>(())
    });
    // Drop the turn once it is demonstrably mid-flight (begin has finished).
    tokio::select! {
        _ = turn => panic!("the turn cannot finish on its own"),
        _ = started_rx => {}
    }
    // The guard finalises on a spawned task.
    for _ in 0..100 {
        let t = only_task(ws.path()).await;
        if t.status == TaskStatus::Failed {
            assert!(t.summary.unwrap().contains("cancelled"));
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("cancelled turn was never finalised");
}

#[tokio::test]
async fn unresolvable_root_is_a_clear_error_not_an_unscoped_fallback() {
    let e = resolve_root(Some("/definitely/not/a/real/path/xyz"))
        .await
        .unwrap_err();
    assert!(e.contains("Debug mode cannot start"), "{e}");
    assert!(e.contains("NEPPY_DEBUG_PROJECT_ROOT"), "{e}");
}

#[test]
fn debug_agent_is_refused_outside_a_debug_turn_and_other_agents_are_not() {
    assert!(ensure_agent_allowed(DEBUG_AGENT_ID).is_err());
    assert!(ensure_agent_allowed("code_executor").is_ok());
    assert!(ensure_agent_allowed("orchestrator").is_ok());
}

#[tokio::test]
async fn debug_agent_is_allowed_inside_a_debug_turn() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let ok = run_in_root(ws.path(), root_of(&repo).await, "x", async {
        Ok::<_, String>(ensure_agent_allowed(DEBUG_AGENT_ID))
    })
    .await
    .unwrap();
    assert!(ok.is_ok());
}

#[tokio::test]
async fn cancel_during_begin_leaves_the_task_failed_not_editing() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = root_of(&repo).await;
    // `begin` records the task synchronously, then parks on its first git
    // call; the ready branch wins on the same poll and drops it mid-flight.
    tokio::select! {
        biased;
        _ = begin(ws.path(), root, "dropped early") => panic!("begin cannot finish in one poll"),
        _ = async {} => {}
    }
    for _ in 0..100 {
        let t = only_task(ws.path()).await;
        if t.status == TaskStatus::Failed {
            assert!(t.summary.unwrap().contains("cancelled"));
            return;
        }
        assert_eq!(
            t.status,
            TaskStatus::Planning,
            "must never linger as Editing"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("task dropped during begin was never finalised");
}

#[tokio::test]
async fn cancel_during_finish_leaves_the_task_failed_not_editing() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = root_of(&repo).await;
    let paused = std::sync::Arc::new(tokio::sync::Notify::new());
    let turn = TEST_PAUSE_IN_FINISH.scope(
        paused.clone(),
        run_in_root(ws.path(), root, "dropped in finish", async {
            Ok::<_, String>(())
        }),
    );
    // `finish` signals and parks; drop the whole turn at that point.
    tokio::select! {
        _ = turn => panic!("finish is parked and cannot complete"),
        _ = paused.notified() => {}
    }
    for _ in 0..100 {
        let t = only_task(ws.path()).await;
        if t.status == TaskStatus::Failed {
            assert!(t.summary.unwrap().contains("cancelled"));
            return;
        }
        assert_eq!(
            t.status,
            TaskStatus::Editing,
            "must not be left as Pass/Partial"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("task dropped mid-finish was never finalised");
}

#[tokio::test]
async fn critical_files_are_recorded_and_an_unvalidated_pass_becomes_partial() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let path = repo.path().to_path_buf();
    run_in_root(ws.path(), root_of(&repo).await, "edit core", async move {
        std::fs::write(path.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(path.join("a.txt"), "two\n").unwrap();
        // The agent claims `pass` without going through `debug_report`'s gate.
        let t = current().unwrap();
        let patch = TaskPatch {
            status: Some(TaskStatus::Pass),
            ..Default::default()
        };
        ops::task_update(&t.ctx, t.task_id.as_deref().unwrap(), patch)
            .await
            .unwrap();
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    let t = only_task(ws.path()).await;
    assert_eq!(t.critical_files, ["Cargo.toml"]);
    assert_eq!(t.files_changed, ["Cargo.toml", "a.txt"]);
    assert_eq!(t.status, TaskStatus::Partial, "no validated candidate");
}

#[tokio::test]
async fn non_critical_pass_is_kept_and_records_no_critical_files() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let path = repo.path().to_path_buf();
    run_in_root(ws.path(), root_of(&repo).await, "edit", async move {
        std::fs::write(path.join("a.txt"), "two\n").unwrap();
        let t = current().unwrap();
        let patch = TaskPatch {
            status: Some(TaskStatus::Pass),
            ..Default::default()
        };
        ops::task_update(&t.ctx, t.task_id.as_deref().unwrap(), patch)
            .await
            .unwrap();
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    let t = only_task(ws.path()).await;
    assert!(t.critical_files.is_empty());
    assert_eq!(t.status, TaskStatus::Pass);
}

#[tokio::test]
async fn without_an_auto_checkpoint_files_changed_is_diffed_against_head() {
    let settings = DebugModeConfig {
        auto_checkpoint: false,
        ..DebugModeConfig::default()
    };
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let path = repo.path().to_path_buf();
    run_in_root_with(
        ws.path(),
        root_of(&repo).await,
        "edit",
        settings.clone(),
        async move {
            std::fs::write(path.join("a.txt"), "two\n").unwrap();
            std::fs::write(path.join("new.txt"), "n\n").unwrap();
            Ok::<_, String>(())
        },
    )
    .await
    .unwrap();
    let t = only_task(ws.path()).await;
    assert!(t.checkpoint_id.is_none());
    assert_eq!(t.files_changed, ["a.txt", "new.txt"]);
    assert_eq!(t.status, TaskStatus::Partial);

    // An untouched tree ends `pass` rather than `partial`.
    let (clean, ws2) = (
        crate::neppy::agent::debug_mode::test_util::repo(),
        tempfile::tempdir().unwrap(),
    );
    run_in_root_with(ws2.path(), root_of(&clean).await, "look", settings, async {
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    let t = only_task(ws2.path()).await;
    assert!(t.files_changed.is_empty());
    assert_eq!(t.status, TaskStatus::Pass);
}
