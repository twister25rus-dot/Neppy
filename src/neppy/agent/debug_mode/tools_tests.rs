use serde_json::json;

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{begin, resolve_root, with_turn};

use crate::neppy::agent::debug_mode::test_util::repo;

async fn in_turn<F: std::future::Future>(
    repo: &tempfile::TempDir,
    ws: &tempfile::TempDir,
    fut: F,
) -> (DebugTurn, F::Output) {
    let root = resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let turn = begin(ws.path(), root, "tool test").await;
    let out = with_turn(turn.clone(), fut).await;
    (turn, out)
}

/// Runs an allowed trivial command through `debug_run_check`, recording a real
/// passing validation on the current task.
async fn run_real_check() {
    let r = DebugRunCheckTool
        .execute(json!({"command": ["git", "rev-parse", "HEAD"]}))
        .await
        .unwrap();
    assert!(!r.is_error, "{}", r.text());
}

#[tokio::test]
async fn tools_refuse_outside_a_debug_turn() {
    let r = DebugCheckpointTool
        .execute(json!({"description": "x"}))
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.text().contains("Debug-mode turn"), "{}", r.text());
    let r = DebugReportTool
        .execute(json!({"status": "pass", "summary": "s"}))
        .await
        .unwrap();
    assert!(r.is_error);
    assert!(r.text().contains("Debug-mode turn"));
}

#[tokio::test]
async fn debug_checkpoint_records_a_checkpoint_for_the_current_task() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let (turn, r) = in_turn(&repo, &ws, async {
        std::fs::write(repo.path().join("a.txt"), "changed\n").unwrap();
        let ok = DebugCheckpointTool
            .execute(json!({"description": "milestone 1"}))
            .await
            .unwrap();
        let empty = DebugCheckpointTool
            .execute(json!({"description": "  "}))
            .await
            .unwrap();
        (ok, empty)
    })
    .await;
    assert!(!r.0.is_error, "{}", r.0.text());
    assert!(r.1.is_error, "blank description rejected");
    let ctx = DebugCtx::new(ws.path());
    let cps = ops::checkpoint_list(&ctx, None).await.unwrap().value;
    let mid = cps
        .iter()
        .find(|c| c.description == "milestone 1")
        .expect("mid-task checkpoint stored");
    assert_eq!(mid.task_id, turn.task_id);
    assert_eq!(mid.dirty_files, ["a.txt"]);
    assert_eq!(cps.len(), 2, "automatic pre-turn checkpoint + mid-task one");
}

#[tokio::test]
async fn debug_report_updates_the_task_and_stays_honest() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let (turn, (bad_status, lying_pass, ok)) = in_turn(&repo, &ws, async {
        let bad_status = DebugReportTool
            .execute(json!({"status": "great", "summary": "s"}))
            .await
            .unwrap();
        let lying_pass = DebugReportTool
            .execute(json!({
                "status": "pass", "summary": "s",
                "validation": [{"check": "pnpm test", "passed": false, "detail": "2 failed"}]
            }))
            .await
            .unwrap();
        let ok = DebugReportTool
            .execute(json!({
                "status": "partial", "summary": "typecheck ok, tests red",
                "validation": [
                    {"check": "pnpm typecheck", "passed": true},
                    {"check": "pnpm test", "passed": false, "detail": "2 failed"}
                ]
            }))
            .await
            .unwrap();
        (bad_status, lying_pass, ok)
    })
    .await;
    assert!(bad_status.is_error);
    assert!(lying_pass.is_error);
    assert!(!ok.is_error, "{}", ok.text());

    let ctx = DebugCtx::new(ws.path());
    let t = ops::task_get(&ctx, turn.task_id.as_deref().unwrap())
        .await
        .unwrap()
        .value;
    assert_eq!(t.status, TaskStatus::Partial);
    assert_eq!(t.summary.as_deref(), Some("typecheck ok, tests red"));
    assert_eq!(t.validation.len(), 2);
    assert!(t.validation[0].passed && !t.validation[1].passed);
    assert_eq!(t.validation[1].output_tail, "2 failed");
}

mod self_mod {
    use super::*;
    use crate::neppy::agent::debug_mode::candidate::{self, CandidateStore};
    use crate::neppy::agent::debug_mode::git::Git;
    use crate::neppy::agent::debug_mode::types::{CandidatePhase, CandidateRecord};

    fn write_critical(repo: &tempfile::TempDir) {
        std::fs::create_dir_all(repo.path().join("src/core")).unwrap();
        std::fs::write(repo.path().join("src/core/x.rs"), "// edit\n").unwrap();
    }

    #[tokio::test]
    async fn validate_candidate_refuses_outside_a_debug_turn() {
        let r = DebugValidateCandidateTool.execute(json!({})).await.unwrap();
        assert!(r.is_error);
        assert!(r.text().contains("Debug-mode turn"), "{}", r.text());
    }

    #[tokio::test]
    async fn pass_is_downgraded_when_critical_files_changed_without_a_candidate() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            write_critical(&repo);
            DebugReportTool
                .execute(json!({"status": "pass", "summary": "touched core"}))
                .await
                .unwrap()
        })
        .await;
        assert!(!r.is_error, "{}", r.text());
        assert!(r.text().contains("DOWNGRADED"), "{}", r.text());
        assert!(r.text().contains("debug_validate_candidate"));
        let task = ops::task_get(&turn.ctx, turn.task_id.as_deref().unwrap())
            .await
            .unwrap()
            .value;
        assert_eq!(task.status, TaskStatus::Partial);
        assert_eq!(task.critical_files, vec!["src/core/x.rs".to_string()]);
    }

    #[tokio::test]
    async fn pass_is_kept_when_a_candidate_passed_for_the_current_tree() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            write_critical(&repo);
            run_real_check().await;
            let tree = Git::new(repo.path())
                .snapshot_trees()
                .await
                .unwrap()
                .worktree_tree;
            let ctx = DebugCtx::new(ws.path());
            CandidateStore::new(&ctx)
                .add(CandidateRecord {
                    id: "cand-1".into(),
                    task_id: None,
                    tree_hash: tree,
                    started_at: "s".into(),
                    finished_at: Some("f".into()),
                    phase: CandidatePhase::Passed,
                    build_ok: true,
                    health_ok: true,
                    error_tail: String::new(),
                    known_good_path: None,
                })
                .unwrap();
            assert!(candidate::candidate_valid_for_current_tree(&ctx, repo.path()).await);
            DebugReportTool
                .execute(json!({"status": "pass", "summary": "validated"}))
                .await
                .unwrap()
        })
        .await;
        assert!(
            !r.is_error && !r.text().contains("DOWNGRADED"),
            "{}",
            r.text()
        );
        let task = ops::task_get(&turn.ctx, turn.task_id.as_deref().unwrap())
            .await
            .unwrap()
            .value;
        assert_eq!(task.status, TaskStatus::Pass);
        assert_eq!(
            task.critical_files.len(),
            1,
            "critical files still recorded"
        );
    }

    #[tokio::test]
    async fn non_critical_changes_pass_without_a_candidate() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            std::fs::write(repo.path().join("a.txt"), "changed\n").unwrap();
            run_real_check().await;
            DebugReportTool
                .execute(json!({"status": "pass", "summary": "docs"}))
                .await
                .unwrap()
        })
        .await;
        assert!(!r.text().contains("DOWNGRADED"), "{}", r.text());
        let task = ops::task_get(&turn.ctx, turn.task_id.as_deref().unwrap())
            .await
            .unwrap()
            .value;
        assert_eq!(task.status, TaskStatus::Pass);
        assert!(task.critical_files.is_empty());
    }
}

mod pass_gate {
    use super::*;
    use crate::neppy::agent::debug_mode::types::TaskStatus;

    async fn status_of(turn: &DebugTurn) -> TaskStatus {
        ops::task_get(&turn.ctx, turn.task_id.as_deref().unwrap())
            .await
            .unwrap()
            .value
            .status
    }

    fn edit(repo: &tempfile::TempDir, body: &str) {
        std::fs::write(repo.path().join("a.txt"), body).unwrap();
    }

    async fn report_pass(extra: serde_json::Value) -> ToolResult {
        let mut args = json!({"status": "pass", "summary": "done"});
        if !extra.is_null() {
            args["validation"] = extra;
        }
        DebugReportTool.execute(args).await.unwrap()
    }

    #[tokio::test]
    async fn pass_is_accepted_after_a_passing_post_edit_check() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            edit(&repo, "two\n");
            run_real_check().await;
            report_pass(json!(null)).await
        })
        .await;
        assert!(
            !r.is_error && !r.text().contains("DOWNGRADED"),
            "{}",
            r.text()
        );
        assert_eq!(status_of(&turn).await, TaskStatus::Pass);
    }

    #[tokio::test]
    async fn pass_is_downgraded_without_any_check() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            edit(&repo, "two\n");
            report_pass(json!(null)).await
        })
        .await;
        assert!(!r.is_error, "downgrade is not an error: {}", r.text());
        assert!(
            r.text().contains(
                "DOWNGRADED from pass to partial: no passing check ran after the last edit"
            ),
            "{}",
            r.text()
        );
        assert!(r.text().contains("debug_run_check"));
        assert_eq!(status_of(&turn).await, TaskStatus::Partial);
    }

    #[tokio::test]
    async fn pass_is_downgraded_when_the_only_check_predates_the_last_edit() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            edit(&repo, "two\n");
            run_real_check().await;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            edit(&repo, "three\n");
            report_pass(json!(null)).await
        })
        .await;
        assert!(r.text().contains("DOWNGRADED"), "{}", r.text());
        assert_eq!(status_of(&turn).await, TaskStatus::Partial);

        // Re-running the check after the edit lets the same report stand, and
        // the real record survives the earlier report.
        let (r2, task) = with_turn(turn.clone(), async {
            run_real_check().await;
            let r = report_pass(json!(null)).await;
            let task = ops::task_get(&turn.ctx, turn.task_id.as_deref().unwrap())
                .await
                .unwrap()
                .value;
            (r, task)
        })
        .await;
        assert!(!r2.text().contains("DOWNGRADED"), "{}", r2.text());
        assert_eq!(task.status, TaskStatus::Pass);
        assert_eq!(
            task.validation
                .iter()
                .filter(|v| !v.command.is_empty())
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn self_attested_validation_never_satisfies_the_gate() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            edit(&repo, "two\n");
            report_pass(json!([{"check": "cargo test", "passed": true}])).await
        })
        .await;
        assert!(r.text().contains("DOWNGRADED"), "{}", r.text());
        let task = ops::task_get(&turn.ctx, turn.task_id.as_deref().unwrap())
            .await
            .unwrap()
            .value;
        assert_eq!(task.status, TaskStatus::Partial);
        assert_eq!(task.validation.len(), 1, "kept as information");
        assert!(task.validation[0].command.is_empty() && task.validation[0].exit_code.is_none());
    }

    #[tokio::test]
    async fn pass_needs_no_check_when_no_files_changed() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async { report_pass(json!(null)).await }).await;
        assert!(!r.text().contains("DOWNGRADED"), "{}", r.text());
        assert_eq!(status_of(&turn).await, TaskStatus::Pass);
    }

    #[tokio::test]
    async fn both_gates_report_when_both_fail() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            std::fs::create_dir_all(repo.path().join("src/core")).unwrap();
            std::fs::write(repo.path().join("src/core/x.rs"), "// edit\n").unwrap();
            report_pass(json!(null)).await
        })
        .await;
        assert!(r.text().contains("critical files changed"), "{}", r.text());
        assert!(r.text().contains("no passing check ran"), "{}", r.text());
        assert_eq!(status_of(&turn).await, TaskStatus::Partial);
    }

    #[tokio::test]
    async fn partial_and_failed_reports_do_not_need_a_check() {
        let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
        let (turn, r) = in_turn(&repo, &ws, async {
            edit(&repo, "two\n");
            DebugReportTool
                .execute(json!({"status": "partial", "summary": "wip"}))
                .await
                .unwrap()
        })
        .await;
        assert!(!r.is_error && !r.text().contains("DOWNGRADED"));
        assert_eq!(status_of(&turn).await, TaskStatus::Partial);
    }
}
