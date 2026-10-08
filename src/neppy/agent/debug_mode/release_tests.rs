use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::*;
use crate::neppy::agent::debug_mode::release_preflight::{
    bump_patch, sanitize_branch, NOTHING_TO_RELEASE, NOT_RELEASE_BRANCH,
};
use crate::neppy::agent::debug_mode::release_steps::GhProbe;
use crate::neppy::agent::debug_mode::test_util::repo;

/// A pid no process can have (above every platform's pid limit).
const DEAD_PID: u32 = 2_000_000_000;

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct FakeGh(AtomicBool);

#[async_trait]
impl GhProbe for FakeGh {
    async fn ready(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct FakeSpawner {
    fail: AtomicBool,
    calls: Mutex<Vec<SpawnRequest>>,
}

impl ReleaseSpawner for FakeSpawner {
    fn spawn(&self, req: &SpawnRequest) -> Result<u32, String> {
        if self.fail.load(Ordering::SeqCst) {
            return Err("cannot start the release process: boom".into());
        }
        self.calls.lock().unwrap().push(req.clone());
        Ok(std::process::id())
    }
}

impl FakeSpawner {
    fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

struct Fixture {
    _origin: tempfile::TempDir,
    root_dir: tempfile::TempDir,
    home: tempfile::TempDir,
    state: tempfile::TempDir,
    gh_ok: Arc<FakeGh>,
}

impl Fixture {
    /// A repo on `main` at version 1.2.3, pushed to a bare `origin`, with one
    /// extra local commit (so there is something to release), a signing key
    /// in a temp HOME and an authenticated `gh`.
    fn new() -> Self {
        let origin = tempfile::tempdir().unwrap();
        sh(origin.path(), &["init", "-q", "--bare"]);
        sh(origin.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
        let root_dir = repo();
        let root = root_dir.path();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(root.join("scripts")).unwrap();
        std::fs::write(root.join("app/package.json"), "{\"version\": \"1.2.3\"}\n").unwrap();
        std::fs::write(
            root.join("scripts/release-neppy.sh"),
            "#!/usr/bin/env bash\nREPO=\"acme/widgets\"\necho fake\n",
        )
        .unwrap();
        sh(root, &["add", "."]);
        sh(root, &["commit", "-q", "-m", "fixture"]);
        sh(
            root,
            &[
                "remote",
                "add",
                "origin",
                &origin.path().display().to_string(),
            ],
        );
        sh(root, &["push", "-q", "origin", "main"]);
        sh(root, &["fetch", "-q", "origin"]);
        sh(root, &["tag", "v1.2.3"]);
        std::fs::write(root.join("feature.txt"), "x\n").unwrap();
        sh(root, &["add", "."]);
        sh(root, &["commit", "-q", "-m", "feat: something new"]);
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".neppy-updater")).unwrap();
        std::fs::write(home.path().join(".neppy-updater/neppy.key"), "not a key").unwrap();
        Self {
            _origin: origin,
            root_dir,
            home,
            state: tempfile::tempdir().unwrap(),
            gh_ok: Arc::new(FakeGh(AtomicBool::new(true))),
        }
    }

    fn root(&self) -> &Path {
        self.root_dir.path()
    }

    fn dir(&self) -> &Path {
        self.state.path()
    }

    fn env(&self) -> ReleaseEnv {
        ReleaseEnv {
            home: Some(self.home.path().to_path_buf()),
            release_branch: "main".into(),
            gh: self.gh_ok.clone(),
        }
    }

    async fn preflight(&self) -> ReleasePreflight {
        preflight_with(self.dir(), self.root(), &self.env())
            .await
            .unwrap()
    }

    async fn start(&self, version: &str, sp: &FakeSpawner) -> Result<ReleaseRecord, String> {
        start_with(self.dir(), self.root(), version, &self.env(), sp).await
    }

    fn store(&self) -> ReleaseStore {
        ReleaseStore {
            dir: self.dir().to_path_buf(),
        }
    }

    /// Persists a record as if a release had been started.
    fn put(&self, phase: ReleasePhase, pid: Option<u32>, started_ago: chrono::Duration) {
        let s = Stored {
            record: ReleaseRecord {
                phase,
                version: Some("1.2.4".into()),
                started_at: Some((chrono::Utc::now() - started_ago).to_rfc3339()),
                ..Default::default()
            },
            pid,
            root: Some(self.root().display().to_string()),
        };
        self.store().write(&s).unwrap();
    }

    fn log(&self, text: &str) {
        std::fs::write(self.dir().join(LOG_FILE), text).unwrap();
    }

    fn exit(&self, text: &str) {
        std::fs::write(self.dir().join(EXIT_FILE), text).unwrap();
    }
}

// ── preflight ────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_ready_repo_has_no_blockers_and_fills_every_field() {
    let f = Fixture::new();
    let p = f.preflight().await;
    assert_eq!(p.blockers, Vec::<String>::new());
    assert_eq!(p.project_root, f.root().display().to_string());
    assert_eq!(p.branch, "main");
    assert_eq!(p.release_branch, "main");
    assert!(p.clean);
    assert!(!p.behind);
    assert_eq!(p.ahead_commits, 1);
    assert_eq!(p.current_version, "1.2.3");
    assert_eq!(p.suggested_version, "1.2.4");
    assert!(p.signing_key_present);
    assert!(p.gh_ready);
    assert_eq!(p.fetch_error, None);
    assert_eq!(p.last_tag.as_deref(), Some("v1.2.3"));
}

#[tokio::test]
async fn another_branch_is_not_the_release_branch() {
    let f = Fixture::new();
    sh(f.root(), &["checkout", "-q", "-b", "feature"]);
    let p = f.preflight().await;
    assert_eq!(p.branch, "feature");
    assert!(p.blockers.contains(&NOT_RELEASE_BRANCH.to_string()));
}

#[tokio::test]
async fn an_uncommitted_file_blocks_as_dirty() {
    let f = Fixture::new();
    std::fs::write(f.root().join("scratch.txt"), "wip").unwrap();
    let p = f.preflight().await;
    assert!(!p.clean);
    assert_eq!(p.blockers, vec!["dirty".to_string()]);
}

#[tokio::test]
async fn new_commits_on_origin_after_a_fetch_block_as_behind() {
    let f = Fixture::new();
    let other = tempfile::tempdir().unwrap();
    sh(
        other.path(),
        &["clone", "-q", &f._origin.path().display().to_string(), "."],
    );
    std::fs::write(other.path().join("remote.txt"), "r").unwrap();
    sh(other.path(), &["add", "."]);
    sh(other.path(), &["commit", "-q", "-m", "remote work"]);
    sh(other.path(), &["push", "-q", "origin", "HEAD:main"]);
    let p = f.preflight().await;
    assert!(p.behind, "preflight fetches before comparing");
    assert_eq!(p.fetch_error, None);
    // Still one local commit ahead: diverged, so only `behind` is reported.
    assert_eq!(p.ahead_commits, 1);
    assert_eq!(p.blockers, vec!["behind".to_string()]);
}

#[tokio::test]
async fn purely_behind_origin_reports_behind_but_not_nothing_to_release() {
    let f = Fixture::new();
    let other = tempfile::tempdir().unwrap();
    sh(
        other.path(),
        &["clone", "-q", &f._origin.path().display().to_string(), "."],
    );
    std::fs::write(other.path().join("remote.txt"), "r").unwrap();
    sh(other.path(), &["add", "."]);
    sh(other.path(), &["commit", "-q", "-m", "remote work"]);
    sh(other.path(), &["push", "-q", "origin", "HEAD:main"]);
    // Drop our own unpublished commit: HEAD is now an ancestor of origin/main.
    sh(f.root(), &["reset", "-q", "--hard", "HEAD~1"]);
    let p = f.preflight().await;
    assert!(p.behind);
    assert_eq!(p.ahead_commits, 0);
    assert_eq!(p.blockers, vec!["behind".to_string()]);
}

#[tokio::test]
async fn head_equal_to_origin_has_nothing_to_release() {
    let f = Fixture::new();
    sh(f.root(), &["push", "-q", "origin", "main"]);
    let p = f.preflight().await;
    assert_eq!(p.ahead_commits, 0);
    assert_eq!(p.blockers, vec![NOTHING_TO_RELEASE.to_string()]);
}

#[tokio::test]
async fn a_missing_signing_key_blocks_without_reading_it() {
    let f = Fixture::new();
    std::fs::remove_file(f.home.path().join(".neppy-updater/neppy.key")).unwrap();
    let p = f.preflight().await;
    assert!(!p.signing_key_present);
    assert_eq!(p.blockers, vec!["no_signing_key".to_string()]);
}

#[tokio::test]
async fn an_unauthenticated_gh_blocks() {
    let f = Fixture::new();
    f.gh_ok.0.store(false, Ordering::SeqCst);
    let p = f.preflight().await;
    assert!(!p.gh_ready);
    assert_eq!(p.blockers, vec!["gh_not_ready".to_string()]);
}

#[tokio::test]
async fn a_running_release_blocks_a_second_one() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(std::process::id()),
        chrono::Duration::minutes(1),
    );
    let p = f.preflight().await;
    assert_eq!(p.blockers, vec!["release_running".to_string()]);
}

#[tokio::test]
async fn a_failed_fetch_is_reported_and_local_refs_are_still_used() {
    let f = Fixture::new();
    sh(
        f.root(),
        &["remote", "set-url", "origin", "/nonexistent/origin.git"],
    );
    let p = f.preflight().await;
    let err = p.fetch_error.expect("fetch error is surfaced");
    assert!(!err.is_empty() && err.len() <= 300, "{err}");
    assert_eq!(
        p.ahead_commits, 1,
        "computed from the local origin/main ref"
    );
    assert!(!p.behind);
}

#[test]
fn versions_parse_bump_and_branches_sanitize() {
    assert_eq!(bump_patch("1.2.3").as_deref(), Some("1.2.4"));
    assert_eq!(bump_patch("0.67.9").as_deref(), Some("0.67.10"));
    assert_eq!(bump_patch("1.2"), None);
    assert_eq!(bump_patch("1.2.3-beta"), None);
    assert_eq!(parse_semver("10.0.1"), Some((10, 0, 1)));
    assert_eq!(parse_semver("1.2.3.4"), None);
    assert_eq!(parse_semver("1..3"), None);
    assert_eq!(parse_semver("+1.2.3"), None);
    assert_eq!(sanitize_branch(""), "main");
    assert_eq!(sanitize_branch("release/0.x"), "release/0.x");
    assert_eq!(sanitize_branch("--upload-pack=evil"), "main");
    assert_eq!(sanitize_branch("a b"), "main");
}

// ── start ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn start_rejects_bad_versions_before_anything_runs() {
    let f = Fixture::new();
    let sp = FakeSpawner::default();
    for bad in [
        "",
        "1.2",
        "v1.2.4",
        "1.2.4-rc1",
        "1.2.4 ",
        "a.b.c",
        "1.2.3",
        "1.2.2",
        "0.9.9",
    ] {
        assert!(f.start(bad, &sp).await.is_err(), "'{bad}' must be refused");
    }
    let e = f.start("1.2.3", &sp).await.unwrap_err();
    assert!(e.contains("greater"), "{e}");
    let e = f.start("1.2", &sp).await.unwrap_err();
    assert!(e.contains("X.Y.Z"), "{e}");
    assert_eq!(sp.count(), 0);
    assert_eq!(status_in(f.dir()).unwrap().phase, ReleasePhase::Idle);
}

#[tokio::test]
async fn start_refuses_while_a_blocker_stands_and_names_it() {
    let f = Fixture::new();
    std::fs::write(f.root().join("scratch.txt"), "wip").unwrap();
    let sp = FakeSpawner::default();
    let e = f.start("1.2.4", &sp).await.unwrap_err();
    assert!(e.contains("dirty"), "{e}");
    assert_eq!(sp.count(), 0);
    assert_eq!(status_in(f.dir()).unwrap().phase, ReleasePhase::Idle);
}

#[tokio::test]
async fn start_records_running_before_returning_and_spawns_the_launcher() {
    let f = Fixture::new();
    f.log("stale output from a previous release\n");
    f.exit("1\n");
    let sp = FakeSpawner::default();
    let rec = f.start("1.2.4", &sp).await.unwrap();
    assert_eq!(rec.phase, ReleasePhase::Running);
    assert_eq!(rec.version.as_deref(), Some("1.2.4"));
    assert!(rec.started_at.is_some());

    let stored = f.store().read().unwrap();
    assert_eq!(stored.record.phase, ReleasePhase::Running);
    assert_eq!(stored.pid, Some(std::process::id()));

    let calls = sp.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    let req = &calls[0];
    let exit_path = f.dir().join(EXIT_FILE);
    assert_eq!(
        req.argv,
        vec![
            "bash".to_string(),
            "-c".into(),
            LAUNCH_SCRIPT.into(),
            "_".into(),
            "1.2.4".into(),
            exit_path.display().to_string()
        ]
    );
    assert!(LAUNCH_SCRIPT.contains("scripts/release-neppy.sh"));
    assert_eq!(req.cwd, f.root());
    assert_eq!(req.log_path, f.dir().join(LOG_FILE));
    let env = |k: &str| req.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(env("GGML_NATIVE").as_deref(), Some("OFF"));
    assert!(env("PATH").unwrap().contains("/opt/homebrew/bin"));
    assert!(
        env("TAURI_SIGNING_PRIVATE_KEY").is_none(),
        "the script sets its own"
    );
    assert_eq!(
        std::fs::read_to_string(&req.log_path).unwrap(),
        "",
        "log truncated"
    );
    assert!(!exit_path.exists(), "a stale exit code must not leak in");
}

#[tokio::test]
async fn start_refuses_a_second_release_while_one_runs() {
    let f = Fixture::new();
    let sp = FakeSpawner::default();
    f.start("1.2.4", &sp).await.unwrap();
    let e = f.start("1.2.5", &sp).await.unwrap_err();
    assert!(
        e.contains("release_running") || e.contains("already running"),
        "{e}"
    );
    assert_eq!(sp.count(), 1);
}

#[tokio::test]
async fn a_launcher_that_cannot_start_leaves_a_failed_record() {
    let f = Fixture::new();
    let sp = FakeSpawner::default();
    sp.fail.store(true, Ordering::SeqCst);
    let e = f.start("1.2.4", &sp).await.unwrap_err();
    assert!(e.contains("boom"), "{e}");
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.phase, ReleasePhase::Failed);
    assert!(rec.error.contains("boom"));
    sp.fail.store(false, Ordering::SeqCst);
    f.start("1.2.4", &sp)
        .await
        .expect("a failed start does not wedge the next");
}

// ── status / reconcile ───────────────────────────────────────────────────

#[test]
fn no_record_is_idle_with_the_contract_defaults() {
    let d = tempfile::tempdir().unwrap();
    let rec = status_in(d.path()).unwrap();
    assert_eq!(rec, ReleaseRecord::default());
    assert_eq!(rec.phase, ReleasePhase::Idle);
    assert_eq!(rec.log_tail, "");
    assert_eq!(rec.error, "");
}

#[test]
fn a_live_recent_release_stays_running_with_its_log() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(std::process::id()),
        chrono::Duration::minutes(5),
    );
    f.log("==> building the bundle\n");
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.phase, ReleasePhase::Running);
    assert!(rec.log_tail.contains("building the bundle"));
    assert_eq!(rec.finished_at, None);
}

#[test]
fn exit_zero_becomes_succeeded_with_tag_and_release_url() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(30),
    );
    f.log("done: v1.2.4 published with release notes and its signed app assets.\n");
    f.exit("0\n");
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.phase, ReleasePhase::Succeeded);
    assert_eq!(rec.exit_code, Some(0));
    assert_eq!(rec.tag.as_deref(), Some("v1.2.4"));
    assert_eq!(
        rec.release_url.as_deref(),
        Some("https://github.com/acme/widgets/releases/tag/v1.2.4")
    );
    assert!(rec.finished_at.is_some());
    assert_eq!(rec.error, "");
    // Persisted: a later read says the same without re-deciding.
    assert_eq!(status_in(f.dir()).unwrap().phase, ReleasePhase::Succeeded);
}

#[test]
fn the_release_url_falls_back_to_the_origin_remote() {
    let f = Fixture::new();
    std::fs::write(
        f.root().join("scripts/release-neppy.sh"),
        "echo no repo line\n",
    )
    .unwrap();
    sh(
        f.root(),
        &["remote", "set-url", "origin", "git@github.com:foo/bar.git"],
    );
    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(1),
    );
    f.exit("0");
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(
        rec.release_url.as_deref(),
        Some("https://github.com/foo/bar/releases/tag/v1.2.4")
    );
}

#[test]
fn remote_urls_and_script_lines_parse_to_a_slug() {
    for (url, want) in [
        ("git@github.com:foo/bar.git", Some("foo/bar")),
        ("https://github.com/foo/bar", Some("foo/bar")),
        ("https://user:tok@github.com/foo/bar.git", Some("foo/bar")),
        ("ssh://git@github.com/foo/bar.git/", Some("foo/bar")),
        ("https://gitlab.com/foo/bar.git", None),
        ("https://github.com/foo", None),
        ("https://github.com/foo/bar/baz", None),
    ] {
        assert_eq!(slug_from_remote(url).as_deref(), want, "{url}");
    }
    assert!(valid_slug("twister25rus-dot/Neppy"));
    assert!(!valid_slug("a/b/c") && !valid_slug("a b/c") && !valid_slug("/c"));
}

#[test]
fn a_nonzero_exit_becomes_failed_with_the_masked_last_line() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(3),
    );
    f.log("==> refreshing main\nerror: tag v1.2.4 already exists\n\n");
    f.exit("1\n");
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.phase, ReleasePhase::Failed);
    assert_eq!(rec.exit_code, Some(1));
    assert_eq!(rec.error, "error: tag v1.2.4 already exists");
    assert_eq!(rec.tag, None);
    assert_eq!(rec.release_url, None);

    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(3),
    );
    f.log("step ok\nTAURI_SIGNING_PRIVATE_KEY=hunter2hunter2hunter2\n");
    f.exit("2");
    let rec = status_in(f.dir()).unwrap();
    assert!(!rec.error.contains("hunter2"), "{}", rec.error);
    assert!(!rec.log_tail.contains("hunter2"), "{}", rec.log_tail);
    assert!(rec.error.contains(super::super::secrets::MASK));
}

#[test]
fn a_long_last_line_is_trimmed_for_the_error() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(3),
    );
    f.log(&format!("error: {}\n", "x".repeat(2000)));
    f.exit("1");
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.error.chars().count(), 300);
}

#[test]
fn a_dead_process_without_an_exit_code_is_failed() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(3),
    );
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.phase, ReleasePhase::Failed);
    assert_eq!(
        rec.error,
        "the release process exited without reporting a result"
    );
    assert_eq!(rec.exit_code, None);
    // A record that never got a pid is the same case.
    f.put(ReleasePhase::Running, None, chrono::Duration::minutes(3));
    assert_eq!(status_in(f.dir()).unwrap().phase, ReleasePhase::Failed);
}

#[test]
fn a_release_over_two_hours_is_failed_but_not_killed() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(std::process::id()),
        chrono::Duration::minutes(121),
    );
    let rec = status_in(f.dir()).unwrap();
    assert_eq!(rec.phase, ReleasePhase::Failed);
    assert!(rec.error.contains("2 hours"), "{}", rec.error);
    assert!(pid_alive(std::process::id()), "nothing was signalled");
    f.put(
        ReleasePhase::Running,
        Some(std::process::id()),
        chrono::Duration::minutes(119),
    );
    assert_eq!(status_in(f.dir()).unwrap().phase, ReleasePhase::Running);
}

#[test]
fn the_log_tail_is_capped_and_secret_lines_are_masked() {
    let f = Fixture::new();
    f.put(
        ReleasePhase::Running,
        Some(std::process::id()),
        chrono::Duration::minutes(1),
    );
    let mut log = String::from("API_TOKEN=abcdef0123456789abcdef\n");
    log.push_str(&"filler line of build output\n".repeat(1000));
    log.push_str("SIGNING_SECRET=topsecretvalue99\nlast visible line\n");
    f.log(&log);
    let rec = status_in(f.dir()).unwrap();
    assert!(rec.log_tail.len() <= 6 * 1024, "{}", rec.log_tail.len());
    assert!(rec.log_tail.ends_with("last visible line"));
    assert!(!rec.log_tail.contains("topsecretvalue99"));
    let creds = redact_urls("remote: https://user:ghp_abc123@github.com/x/y.git failed");
    assert!(!creds.contains("ghp_abc123"), "{creds}");
}

// ── wire shape ───────────────────────────────────────────────────────────

fn keys(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[tokio::test]
async fn json_field_names_match_the_typescript_contract() {
    let f = Fixture::new();
    let pre = serde_json::to_value(f.preflight().await).unwrap();
    assert_eq!(
        keys(&pre),
        [
            "ahead_commits",
            "behind",
            "blockers",
            "branch",
            "clean",
            "current_version",
            "fetch_error",
            "gh_ready",
            "last_tag",
            "project_root",
            "release_branch",
            "signing_key_present",
            "suggested_version"
        ]
    );
    assert!(pre["fetch_error"].is_null());
    assert!(pre["blockers"].is_array() && pre["ahead_commits"].is_u64());

    let idle = serde_json::to_value(status_in(f.dir()).unwrap()).unwrap();
    let record_keys = [
        "error",
        "exit_code",
        "finished_at",
        "log_tail",
        "phase",
        "release_url",
        "started_at",
        "tag",
        "version",
    ];
    assert_eq!(keys(&idle), record_keys);
    assert_eq!(idle["phase"], "idle");
    assert!(idle["version"].is_null() && idle["exit_code"].is_null());

    f.put(
        ReleasePhase::Running,
        Some(DEAD_PID),
        chrono::Duration::minutes(1),
    );
    f.exit("0");
    let ok = serde_json::to_value(status_in(f.dir()).unwrap()).unwrap();
    assert_eq!(keys(&ok), record_keys, "pid and root never reach the wire");
    assert_eq!(ok["phase"], "succeeded");
    assert_eq!(ok["exit_code"], 0);
}

// ── the real launcher, against a throwaway script ────────────────────────

#[cfg(unix)]
#[test]
fn the_real_launcher_logs_output_and_records_the_exit_code() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("scripts")).unwrap();
    std::fs::write(
        root.path().join("scripts/release-neppy.sh"),
        "echo \"version=$1\"; echo oops >&2; exit 3\n",
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let exit_path = dir.path().join(EXIT_FILE);
    let req = SpawnRequest {
        argv: vec![
            "bash".into(),
            "-c".into(),
            LAUNCH_SCRIPT.into(),
            "_".into(),
            "9.9.9".into(),
            exit_path.display().to_string(),
        ],
        cwd: root.path().to_path_buf(),
        env: vec![("PATH".into(), build_path())],
        log_path: dir.path().join(LOG_FILE),
    };
    let pid = ShellReleaseSpawner.spawn(&req).unwrap();
    assert!(pid > 0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !exit_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_to_string(&exit_path).unwrap().trim(), "3");
    let log = std::fs::read_to_string(&req.log_path).unwrap();
    assert!(
        log.contains("version=9.9.9") && log.contains("oops"),
        "{log}"
    );
}

#[cfg(unix)]
#[test]
fn pid_alive_tells_live_from_dead_and_refuses_group_pids() {
    assert!(pid_alive(std::process::id()));
    assert!(!pid_alive(DEAD_PID));
    assert!(!pid_alive(0));
    assert!(!pid_alive(u32::MAX));
}
