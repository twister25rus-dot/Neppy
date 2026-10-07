use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::neppy::agent::debug_mode::candidate_steps::Launched;
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::agent::debug_mode::types::TaskRecord;

#[derive(Clone)]
struct Fake {
    build: Result<(), String>,
    launch: Result<(), String>,
    health: Result<(), String>,
    build_delay: Duration,
}

impl Fake {
    fn ok() -> Self {
        Self {
            build: Ok(()),
            launch: Ok(()),
            health: Ok(()),
            build_delay: Duration::ZERO,
        }
    }
}

#[async_trait]
impl Steps for Fake {
    async fn build(&self) -> Result<(), String> {
        tokio::time::sleep(self.build_delay).await;
        self.build.clone()
    }
    async fn launch(&self) -> Result<Launched, String> {
        self.launch.clone().map(|_| Launched::inert())
    }
    async fn health(&self, _l: &mut Launched) -> Result<(), String> {
        self.health.clone()
    }
    async fn stop(&self, _l: Launched) {}
}

const POLL: Duration = Duration::from_millis(20);
const LONG: Duration = Duration::from_secs(10);

fn ctx() -> (tempfile::TempDir, DebugCtx) {
    let ws = tempfile::tempdir().unwrap();
    let ctx = DebugCtx::new(ws.path());
    (ws, ctx)
}

fn go(ctx: &DebugCtx, fake: Fake, tree: &str, src: &Path) -> Result<CandidateRecord, String> {
    start_with(ctx, tree.into(), None, Arc::new(fake), src.to_path_buf())
}

#[tokio::test]
async fn a_healthy_candidate_passes_and_preserves_the_current_binary() {
    let (ws, ctx) = ctx();
    let bin = ws.path().join("current-neppy-core");
    fs::write(&bin, b"old binary").unwrap();
    let rec = go(&ctx, Fake::ok(), "tree-a", &bin).unwrap();
    assert_eq!(rec.phase, CandidatePhase::Building);
    let done = wait(&ctx, &rec.id, LONG, POLL).await.unwrap();
    assert_eq!(done.phase, CandidatePhase::Passed);
    assert!(done.build_ok && done.health_ok);
    assert!(done.finished_at.is_some());
    let kg = PathBuf::from(done.known_good_path.expect("known-good recorded"));
    assert_eq!(fs::read(&kg).unwrap(), b"old binary");
    assert!(kg.starts_with(ctx.store.dir().join(KNOWN_GOOD_DIR)));
    assert_eq!(fs::read(&bin).unwrap(), b"old binary", "source untouched");
}

#[tokio::test]
async fn a_missing_current_binary_still_passes_without_known_good() {
    let (ws, ctx) = ctx();
    go(&ctx, Fake::ok(), "t", &ws.path().join("absent")).unwrap();
    let rec = CandidateStore::new(&ctx).latest().unwrap().unwrap();
    let done = wait(&ctx, &rec.id, LONG, POLL).await.unwrap();
    assert_eq!(done.phase, CandidatePhase::Passed);
    assert!(done.known_good_path.is_none());
}

#[tokio::test]
async fn build_failure_fails_without_launching() {
    let (ws, ctx) = ctx();
    let fake = Fake {
        build: Err("error[E0432]: unresolved import".into()),
        launch: Err("must not be reached".into()),
        ..Fake::ok()
    };
    let rec = go(&ctx, fake, "t", &ws.path().join("x")).unwrap();
    let done = wait(&ctx, &rec.id, LONG, POLL).await.unwrap();
    assert_eq!(done.phase, CandidatePhase::Failed);
    assert!(!done.build_ok && !done.health_ok);
    assert!(done.error_tail.contains("E0432"));
    assert!(done.known_good_path.is_none());
}

#[tokio::test]
async fn launch_and_health_failures_keep_build_ok_and_never_pass() {
    for (launch, health, needle) in [
        (Err("spawn failed".to_string()), Ok(()), "spawn failed"),
        (Ok(()), Err("health timed out".to_string()), "timed out"),
    ] {
        let (ws, ctx) = ctx();
        let bin = ws.path().join("cur");
        fs::write(&bin, b"x").unwrap();
        let fake = Fake {
            launch,
            health,
            ..Fake::ok()
        };
        let rec = go(&ctx, fake, "t", &bin).unwrap();
        let done = wait(&ctx, &rec.id, LONG, POLL).await.unwrap();
        assert_eq!(done.phase, CandidatePhase::Failed);
        assert!(done.build_ok && !done.health_ok);
        assert!(done.error_tail.contains(needle), "{}", done.error_tail);
        assert!(
            !ctx.store.dir().join(KNOWN_GOOD_DIR).exists(),
            "nothing preserved for a failed candidate"
        );
    }
}

#[tokio::test]
async fn only_one_candidate_runs_at_a_time() {
    let (ws, ctx) = ctx();
    let slow = Fake {
        build_delay: Duration::from_millis(400),
        ..Fake::ok()
    };
    let first = go(&ctx, slow, "t1", &ws.path().join("x")).unwrap();
    let err = go(&ctx, Fake::ok(), "t2", &ws.path().join("x")).unwrap_err();
    assert!(err.contains("already running"), "{err}");
    wait(&ctx, &first.id, LONG, POLL).await.unwrap();
    // The slot frees once the pipeline ends.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(go(&ctx, Fake::ok(), "t2", &ws.path().join("x")).is_ok());
}

#[tokio::test]
async fn cancel_marks_cancelled_and_frees_the_slot() {
    let (ws, ctx) = ctx();
    let slow = Fake {
        build_delay: Duration::from_secs(60),
        ..Fake::ok()
    };
    let rec = go(&ctx, slow, "t", &ws.path().join("x")).unwrap();
    let cancelled = cancel(&ctx).await.unwrap().value;
    assert_eq!(cancelled.id, rec.id);
    assert_eq!(cancelled.phase, CandidatePhase::Cancelled);
    let again = cancel(&ctx).await.unwrap_err();
    assert!(again.contains("no candidate is running"), "{again}");
    assert!(go(&ctx, Fake::ok(), "t2", &ws.path().join("x")).is_ok());
    let kept = CandidateStore::new(&ctx).get(&rec.id).unwrap().unwrap();
    assert_eq!(kept.phase, CandidatePhase::Cancelled, "stays cancelled");
}

#[tokio::test]
async fn a_stale_running_record_is_failed_as_interrupted() {
    let (_ws, ctx) = ctx();
    let store = CandidateStore::new(&ctx);
    store
        .add(CandidateRecord {
            id: "cand-stale".into(),
            task_id: None,
            tree_hash: "t".into(),
            started_at: "2026-10-01T00:00:00Z".into(),
            finished_at: None,
            phase: CandidatePhase::Building,
            build_ok: false,
            health_ok: false,
            error_tail: String::new(),
            known_good_path: None,
        })
        .unwrap();
    let st = status(&ctx, None).await.unwrap().value.unwrap();
    assert_eq!(st.id, "cand-stale");
    assert_eq!(st.phase, CandidatePhase::Failed);
    assert!(st.error_tail.contains("interrupted"));
}

#[tokio::test]
async fn status_defaults_to_latest_and_rejects_unknown_ids() {
    let (ws, ctx) = ctx();
    assert!(status(&ctx, None).await.unwrap().value.is_none());
    let a = go(&ctx, Fake::ok(), "a", &ws.path().join("x")).unwrap();
    wait(&ctx, &a.id, LONG, POLL).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let b = go(&ctx, Fake::ok(), "b", &ws.path().join("x")).unwrap();
    wait(&ctx, &b.id, LONG, POLL).await.unwrap();
    assert_eq!(status(&ctx, None).await.unwrap().value.unwrap().id, b.id);
    assert_eq!(
        status(&ctx, Some(&a.id))
            .await
            .unwrap()
            .value
            .unwrap()
            .tree_hash,
        "a"
    );
    assert!(status(&ctx, Some("nope")).await.is_err());
}

#[tokio::test]
async fn a_candidate_is_linked_to_its_task_and_unknown_tasks_are_refused() {
    let (ws, ctx) = ctx();
    let task = ops::task_start(&ctx, "self-mod").await.unwrap().value;
    let rec = start_with(
        &ctx,
        "t".into(),
        Some(task.id.clone()),
        Arc::new(Fake::ok()),
        ws.path().join("x"),
    )
    .unwrap();
    assert_eq!(rec.task_id.as_deref(), Some(task.id.as_str()));
    let linked = ctx.store.task_get(&task.id).unwrap().unwrap();
    assert_eq!(linked.candidate_id.as_deref(), Some(rec.id.as_str()));
    wait(&ctx, &rec.id, LONG, POLL).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let err = start_with(
        &ctx,
        "t".into(),
        Some("task-missing".into()),
        Arc::new(Fake::ok()),
        ws.path().join("x"),
    )
    .unwrap_err();
    assert!(err.contains("unknown task"), "{err}");
}

fn passed(id: &str, tree: &str, phase: CandidatePhase) -> CandidateRecord {
    CandidateRecord {
        id: id.into(),
        task_id: None,
        tree_hash: tree.into(),
        started_at: "2026-10-01T00:00:00Z".into(),
        finished_at: Some("2026-10-01T00:01:00Z".into()),
        phase,
        build_ok: true,
        health_ok: phase == CandidatePhase::Passed,
        error_tail: String::new(),
        known_good_path: None,
    }
}

#[test]
fn only_a_passed_candidate_for_the_same_tree_is_valid() {
    let (_ws, ctx) = ctx();
    let store = CandidateStore::new(&ctx);
    store
        .add(passed("c1", "tree-a", CandidatePhase::Passed))
        .unwrap();
    store
        .add(passed("c2", "tree-b", CandidatePhase::Failed))
        .unwrap();
    assert!(valid_for_tree(&ctx, "tree-a"));
    assert!(!valid_for_tree(&ctx, "tree-b"), "failed does not count");
    assert!(!valid_for_tree(&ctx, "tree-c"));
}

#[tokio::test]
async fn validity_tracks_the_real_working_tree() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let ctx = DebugCtx::new(ws.path());
    let tree = Git::new(repo.path())
        .snapshot_trees()
        .await
        .unwrap()
        .worktree_tree;
    CandidateStore::new(&ctx)
        .add(passed("c1", &tree, CandidatePhase::Passed))
        .unwrap();
    assert!(candidate_valid_for_current_tree(&ctx, repo.path()).await);
    fs::write(repo.path().join("a.txt"), "edited after validation\n").unwrap();
    assert!(!candidate_valid_for_current_tree(&ctx, repo.path()).await);
    fs::write(repo.path().join("a.txt"), "one\n").unwrap();
    assert!(
        candidate_valid_for_current_tree(&ctx, repo.path()).await,
        "reverting restores validity"
    );
}

#[test]
fn known_good_keeps_only_the_newest_three() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("neppy-core");
    let dest = dir.path().join("kg");
    let mut last = None;
    for i in 0..5 {
        fs::write(&src, format!("v{i}")).unwrap();
        last = preserve_known_good(&src, &dest).unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut names: Vec<_> = fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), KEEP_KNOWN_GOOD, "{names:?}");
    assert!(
        names.iter().all(|n| n.starts_with("neppy-core-")),
        "{names:?}"
    );
    assert_eq!(fs::read_to_string(last.unwrap()).unwrap(), "v4");
    let oldest = fs::read_to_string(dest.join(&names[0])).unwrap();
    assert_eq!(oldest, "v2");
}

#[test]
fn old_history_files_without_the_new_fields_still_load() {
    let old = r#"{"id":"task-1","request":"r","created_at":"c","updated_at":"u",
        "status":"pass","files_changed":["a"],"validation":[],"summary":null,
        "checkpoint_id":null,"branch":null,"commit":null}"#;
    let t: TaskRecord = serde_json::from_str(old).unwrap();
    assert!(t.critical_files.is_empty());
    assert!(t.candidate_id.is_none());

    let ws = tempfile::tempdir().unwrap();
    let ctx = DebugCtx::new(ws.path());
    fs::create_dir_all(ctx.store.dir()).unwrap();
    fs::write(ctx.store.dir().join("history.json"), format!("[{old}]")).unwrap();
    let tasks = ctx.store.task_list(10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "task-1");
    assert!(tasks[0].candidate_id.is_none());
}

#[test]
fn candidate_records_round_trip_and_tolerate_missing_optional_fields() {
    let r = passed("c", "t", CandidatePhase::HealthCheck);
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("\"health_check\""));
    let minimal =
        r#"{"id":"c","task_id":null,"tree_hash":"t","started_at":"s","phase":"building"}"#;
    let m: CandidateRecord = serde_json::from_str(minimal).unwrap();
    assert!(m.phase.is_running() && !m.build_ok && m.finished_at.is_none());
}

/// Manual smoke test of the real launch + health path (no cargo build):
/// `NEPPY_CANDIDATE_SMOKE_TARGET=<cargo target dir holding debug/neppy-core>
/// cargo test --lib candidate::tests::real_launch -- --ignored`.
#[tokio::test]
#[ignore = "needs a built neppy-core; set NEPPY_CANDIDATE_SMOKE_TARGET"]
async fn real_launch_and_health_against_a_built_binary() {
    let target = std::env::var("NEPPY_CANDIDATE_SMOKE_TARGET").expect("target dir");
    let steps = crate::neppy::agent::debug_mode::candidate_steps::CargoSteps {
        root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        target_dir: PathBuf::from(target),
    };
    let mut launched = steps.launch().await.unwrap();
    let health = steps.health(&mut launched).await;
    steps.stop(launched).await;
    health.unwrap();
}
