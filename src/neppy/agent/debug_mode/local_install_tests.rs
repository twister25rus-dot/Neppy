use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::*;
use crate::neppy::agent::debug_mode::local_install_steps::{tauri_args, TauriSteps};
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::agent::debug_mode::types::{TaskRecord, TaskStatus};

struct Fake {
    result: Result<PathBuf, String>,
    delay: Duration,
}

#[async_trait]
impl InstallSteps for Fake {
    async fn build(&self, log: &Path) -> Result<PathBuf, String> {
        fs::write(log, "compiling neppy...\nlinking\n").unwrap();
        tokio::time::sleep(self.delay).await;
        self.result.clone()
    }
}

#[derive(Default)]
struct FakeSpawner {
    fail: AtomicBool,
    pid: AtomicU32,
    calls: Mutex<Vec<(PathBuf, PathBuf)>>,
}

impl HelperSpawner for FakeSpawner {
    fn spawn(&self, script: &Path, new_app: &Path, app_pid: u32) -> Result<(), String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("cannot start the installer helper: boom".into());
        }
        self.pid.store(app_pid, Ordering::SeqCst);
        lock(&self.calls).push((script.to_path_buf(), new_app.to_path_buf()));
        Ok(())
    }
}

fn ctx() -> (tempfile::TempDir, DebugCtx) {
    let ws = tempfile::tempdir().unwrap();
    let ctx = DebugCtx::new(ws.path());
    (ws, ctx)
}

/// A fake bundle and a project root holding the helper script.
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let bundle = d.path().join("Neppy.app");
    fs::create_dir_all(bundle.join("Contents")).unwrap();
    fs::write(bundle.join("Contents/Info.plist"), "<plist/>").unwrap();
    let root = d.path().join("root");
    fs::create_dir_all(root.join("scripts")).unwrap();
    fs::write(root.join(HELPER_SCRIPT), "#!/bin/bash\n").unwrap();
    (d, bundle, root)
}

async fn settle(ctx: &DebugCtx) -> LocalInstallRecord {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let rec = status_in(ctx.store.dir(), std::process::id()).unwrap();
        if rec.phase != LocalInstallPhase::Building || Instant::now() > deadline {
            return rec;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn start_fake(
    ctx: &DebugCtx,
    result: Result<PathBuf, String>,
    delay: Duration,
) -> Result<LocalInstallRecord, String> {
    start_with(ctx, Some("1.2.3".into()), Arc::new(Fake { result, delay }))
}

#[tokio::test]
async fn a_successful_build_becomes_ready_with_its_bundle_and_log() {
    let (_ws, ctx) = ctx();
    let (_d, bundle, _root) = fixture();
    let rec = start_fake(&ctx, Ok(bundle.clone()), Duration::ZERO).unwrap();
    assert_eq!(rec.phase, LocalInstallPhase::Building);
    assert_eq!(rec.version.as_deref(), Some("1.2.3"));
    let done = settle(&ctx).await;
    assert_eq!(done.phase, LocalInstallPhase::Ready);
    assert_eq!(done.bundle_path, Some(bundle.display().to_string()));
    assert!(done.finished_at.is_some());
    assert!(
        done.log_tail.contains("linking"),
        "log tail: {:?}",
        done.log_tail
    );
}

#[tokio::test]
async fn a_failed_build_records_the_error_tail() {
    let (_ws, ctx) = ctx();
    start_fake(
        &ctx,
        Err("tauri build exited with Some(1)\nerror[E0432]".into()),
        Duration::ZERO,
    )
    .unwrap();
    let done = settle(&ctx).await;
    assert_eq!(done.phase, LocalInstallPhase::Failed);
    assert!(done.error.contains("E0432"));
    assert!(done.bundle_path.is_none());
}

#[tokio::test]
async fn a_second_build_is_refused_while_one_runs_then_allowed() {
    let (_ws, ctx) = ctx();
    let (_d, bundle, _root) = fixture();
    start_fake(&ctx, Ok(bundle.clone()), Duration::from_millis(300)).unwrap();
    let err = start_fake(&ctx, Ok(bundle.clone()), Duration::ZERO).unwrap_err();
    assert!(err.contains("already running"), "{err}");
    assert_eq!(settle(&ctx).await.phase, LocalInstallPhase::Ready);
    start_fake(&ctx, Ok(bundle), Duration::ZERO).expect("a finished build does not block the next");
    settle(&ctx).await;
}

#[test]
fn a_building_record_with_no_live_pipeline_is_marked_failed() {
    let (_ws, ctx) = ctx();
    let store = InstallStore {
        dir: ctx.store.dir().to_path_buf(),
    };
    store
        .modify(|r| r.phase = LocalInstallPhase::Building)
        .unwrap();
    let rec = status_in(ctx.store.dir(), 1).unwrap();
    assert_eq!(rec.phase, LocalInstallPhase::Failed);
    assert!(rec.error.contains("interrupted"));
}

fn ready_ctx() -> (tempfile::TempDir, DebugCtx, tempfile::TempDir, PathBuf) {
    let (ws, ctx) = ctx();
    let (d, bundle, root) = fixture();
    let store = InstallStore {
        dir: ctx.store.dir().to_path_buf(),
    };
    store
        .modify(|r| {
            r.phase = LocalInstallPhase::Ready;
            r.bundle_path = Some(bundle.display().to_string());
        })
        .unwrap();
    (ws, ctx, d, root)
}

#[test]
fn apply_needs_confirm_and_a_ready_build() {
    let (_ws, ctx, _d, root) = ready_ctx();
    let sp = FakeSpawner::default();
    assert!(apply_with(&ctx, false, &root, 7, &sp)
        .unwrap_err()
        .contains("confirm"));
    assert!(lock(&sp.calls).is_empty());

    let (_ws2, idle) = self::ctx();
    let err = apply_with(&idle, true, &root, 7, &sp).unwrap_err();
    assert!(err.contains("not ready"), "{err}");
    let store = InstallStore {
        dir: idle.store.dir().to_path_buf(),
    };
    for phase in [LocalInstallPhase::Building, LocalInstallPhase::Failed] {
        store.modify(|r| r.phase = phase).unwrap();
        // `Building` with no live pipeline reconciles to Failed first; both refuse.
        assert!(apply_with(&idle, true, &root, 7, &sp).is_err());
    }
    assert!(lock(&sp.calls).is_empty(), "the helper never ran");
}

#[test]
fn apply_hands_the_bundle_to_the_helper_and_enters_installing() {
    let (_ws, ctx, _d, root) = ready_ctx();
    let sp = FakeSpawner::default();
    let pid = std::process::id();
    let rec = apply_with(&ctx, true, &root, pid, &sp).unwrap();
    assert_eq!(rec.phase, LocalInstallPhase::Installing);
    assert_eq!(rec.app_pid, Some(pid));
    assert_eq!(sp.pid.load(Ordering::SeqCst), pid);
    let calls = lock(&sp.calls);
    assert_eq!(calls[0].0, root.join(HELPER_SCRIPT));
    assert_eq!(calls[0].1, PathBuf::from(rec.bundle_path.unwrap()));
    drop(calls);
    // A second apply, or a new build, is refused while installing.
    assert!(apply_with(&ctx, true, &root, pid, &sp).is_err());
    assert!(start_fake(&ctx, Ok(PathBuf::new()), Duration::ZERO)
        .unwrap_err()
        .contains("install is in progress"));
}

#[test]
fn a_helper_that_cannot_start_leaves_the_build_ready() {
    let (_ws, ctx, _d, root) = ready_ctx();
    let sp = FakeSpawner::default();
    sp.fail.store(true, Ordering::SeqCst);
    assert!(apply_with(&ctx, true, &root, 9, &sp)
        .unwrap_err()
        .contains("boom"));
    let rec = status_in(ctx.store.dir(), 9).unwrap();
    assert_eq!(rec.phase, LocalInstallPhase::Ready);
    assert!(rec.app_pid.is_none());
}

#[test]
fn apply_refuses_a_missing_bundle_or_helper() {
    let (_ws, ctx, d, root) = ready_ctx();
    let sp = FakeSpawner::default();
    fs::remove_file(root.join(HELPER_SCRIPT)).unwrap();
    assert!(apply_with(&ctx, true, &root, 1, &sp)
        .unwrap_err()
        .contains("helper missing"));
    fs::write(root.join(HELPER_SCRIPT), "x").unwrap();
    fs::remove_dir_all(d.path().join("Neppy.app")).unwrap();
    assert!(apply_with(&ctx, true, &root, 1, &sp)
        .unwrap_err()
        .contains("bundle is gone"));
}

#[test]
fn installing_ends_when_the_app_restarts_or_the_helper_gave_up() {
    let (_ws, ctx, _d, root) = ready_ctx();
    apply_with(&ctx, true, &root, 100, &FakeSpawner::default()).unwrap();
    // Same app still alive and recently asked: still installing.
    assert_eq!(
        status_in(ctx.store.dir(), 100).unwrap().phase,
        LocalInstallPhase::Installing
    );
    // The app that asked is gone and a new process is asking: install is over.
    assert_eq!(
        status_in(ctx.store.dir(), 101).unwrap().phase,
        LocalInstallPhase::Idle
    );

    let (_ws, ctx, _d, root) = ready_ctx();
    apply_with(&ctx, true, &root, 100, &FakeSpawner::default()).unwrap();
    let old = (chrono::Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
    InstallStore {
        dir: ctx.store.dir().to_path_buf(),
    }
    .modify(|r| r.installing_since = Some(old))
    .unwrap();
    assert_eq!(
        status_in(ctx.store.dir(), 100).unwrap().phase,
        LocalInstallPhase::Ready
    );
}

fn task(id: &str, files: &[&str]) -> TaskRecord {
    TaskRecord {
        id: id.into(),
        request: "r".into(),
        created_at: "t".into(),
        updated_at: "t".into(),
        status: TaskStatus::Editing,
        files_changed: files.iter().map(|s| s.to_string()).collect(),
        validation: vec![],
        summary: None,
        checkpoint_id: None,
        branch: None,
        commit: None,
        critical_files: vec![],
        candidate_id: None,
    }
}

#[tokio::test]
async fn the_guard_refuses_a_critical_task_without_a_passed_candidate() {
    let (_ws, ctx) = ctx();
    let root = repo();
    // No active task: nothing to guard.
    check_guard(&ctx, root.path()).await.unwrap();
    // A non-critical task passes.
    ctx.store
        .task_add(task("t-1", &["src/neppy/tools/x.rs"]))
        .unwrap();
    check_guard(&ctx, root.path()).await.unwrap();
    ctx.store
        .task_modify("t-1", |t| t.status = TaskStatus::Pass)
        .unwrap();
    // A critical one is refused until a candidate passed for the current tree.
    ctx.store
        .task_add(task("t-2", &["src/neppy/agent/debug_mode/ops.rs"]))
        .unwrap();
    let err = check_guard(&ctx, root.path()).await.unwrap_err();
    assert!(
        err.starts_with("refused") && err.contains("debug_mode/ops.rs"),
        "{err}"
    );

    let tree = crate::neppy::agent::debug_mode::git::Git::new(root.path())
        .snapshot_trees()
        .await
        .unwrap()
        .worktree_tree;
    let store = crate::neppy::agent::debug_mode::candidate::CandidateStore::new(&ctx);
    store
        .add(crate::neppy::agent::debug_mode::types::CandidateRecord {
            id: "cand-1".into(),
            task_id: None,
            tree_hash: tree,
            started_at: "t".into(),
            finished_at: None,
            phase: crate::neppy::agent::debug_mode::types::CandidatePhase::Passed,
            build_ok: true,
            health_ok: true,
            error_tail: String::new(),
            known_good_path: None,
        })
        .unwrap();
    check_guard(&ctx, root.path())
        .await
        .expect("a passed candidate lifts the guard");
}

#[test]
fn the_install_script_is_protected_and_critical() {
    assert!(selfmod::is_protected_path("scripts/neppy-install-local.sh"));
    assert!(selfmod::is_critical_path("scripts/neppy-install-local.sh"));
    assert!(!selfmod::is_protected_path(
        "scripts/neppy-install-local.sh.bak"
    ));
}

#[test]
fn the_result_is_read_and_acknowledged_once() {
    let d = tempfile::tempdir().unwrap();
    assert!(result_in(d.path(), false).unwrap().is_none());
    fs::write(
        d.path().join(RESULT_FILE),
        r#"{"status":"restored","version":"0.69.0","backup":"/b/Neppy.app","ts":"2026-10-07T10:00:00Z","reason":"launch marker never cleared"}"#,
    )
    .unwrap();
    let r = result_in(d.path(), false).unwrap().unwrap();
    assert_eq!((r.status.as_str(), r.seen), ("restored", false));
    assert!(result_in(d.path(), true).unwrap().unwrap().seen);
    assert!(result_in(d.path(), false).unwrap().unwrap().seen);
    // A newer verdict is unseen again.
    fs::write(
        d.path().join(RESULT_FILE),
        r#"{"status":"installed","ts":"2026-10-08T10:00:00Z"}"#,
    )
    .unwrap();
    assert!(!result_in(d.path(), false).unwrap().unwrap().seen);
    fs::write(d.path().join(RESULT_FILE), "not json").unwrap();
    assert!(result_in(d.path(), false).unwrap().is_none());
}

#[test]
fn the_tauri_build_needs_no_signing_key_and_builds_only_the_app_bundle() {
    let args = tauri_args();
    assert_eq!(&args[..4], ["build", "--bundles", "app", "--config"]);
    let cfg: serde_json::Value = serde_json::from_str(&args[4]).unwrap();
    assert_eq!(cfg["bundle"]["createUpdaterArtifacts"], false);
    assert_eq!(&args[5..], ["--", "--bin", "Neppy"]);
}

/// Runs the real runner against a stand-in `tauri` that records its env.
#[cfg(unix)]
#[tokio::test]
async fn the_runner_isolates_env_streams_the_log_and_finds_the_bundle() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let bin = root.join("app/node_modules/.bin");
    fs::create_dir_all(&bin).unwrap();
    let target = d.path().join("app-target");
    let script = format!(
        "#!/bin/bash\necho \"args: $*\"\necho \"target=$CARGO_TARGET_DIR ggml=$GGML_NATIVE key=[${{TAURI_SIGNING_PRIVATE_KEY:-}}]\"\n\
         mkdir -p \"$CARGO_TARGET_DIR/release/bundle/macos/Neppy.app/Contents\"\n\
         touch \"$CARGO_TARGET_DIR/release/bundle/macos/Neppy.app/Contents/Info.plist\"\n"
    );
    fs::write(bin.join("tauri"), script).unwrap();
    fs::set_permissions(bin.join("tauri"), fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("TAURI_SIGNING_PRIVATE_KEY", "super-secret");
    let steps = TauriSteps {
        root,
        target_dir: target.clone(),
        timeout: Duration::from_secs(30),
    };
    let log = d.path().join("build.log");
    let bundle = steps.build(&log).await.unwrap();
    std::env::remove_var("TAURI_SIGNING_PRIVATE_KEY");
    assert_eq!(bundle, target.join("release/bundle/macos/Neppy.app"));
    let text = fs::read_to_string(&log).unwrap();
    assert!(
        text.contains("args: build --bundles app --config"),
        "{text}"
    );
    assert!(
        text.contains(&format!("target={} ggml=OFF key=[]", target.display())),
        "{text}"
    );
    assert!(!text.contains("super-secret"));
}

#[cfg(unix)]
#[tokio::test]
async fn the_runner_reports_failure_and_timeout_with_a_tail() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("proj");
    let bin = root.join("app/node_modules/.bin");
    fs::create_dir_all(&bin).unwrap();
    let steps = |timeout| TauriSteps {
        root: root.clone(),
        target_dir: d.path().join("t"),
        timeout,
    };
    let log = d.path().join("build.log");
    // Missing CLI.
    assert!(steps(Duration::from_secs(5))
        .build(&log)
        .await
        .unwrap_err()
        .contains("pnpm install"));
    let put = |body: &str| {
        fs::write(bin.join("tauri"), body).unwrap();
        fs::set_permissions(bin.join("tauri"), fs::Permissions::from_mode(0o755)).unwrap();
    };
    put("#!/bin/bash\necho 'error: linker failed'\nexit 3\n");
    let err = steps(Duration::from_secs(5)).build(&log).await.unwrap_err();
    assert!(
        err.contains("Some(3)") && err.contains("linker failed"),
        "{err}"
    );
    put("#!/bin/bash\nsleep 30\n");
    let started = Instant::now();
    let err = steps(Duration::from_millis(300))
        .build(&log)
        .await
        .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(10));
    // Exit 0 but no bundle.
    put("#!/bin/bash\nexit 0\n");
    assert!(steps(Duration::from_secs(5))
        .build(&log)
        .await
        .unwrap_err()
        .contains("is missing"));
}
