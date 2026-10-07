use std::path::Path;
use std::time::Duration;

use super::*;
use crate::neppy::agent::debug_mode::exec::{self, Keep, RunSpec};

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Repo on `main` with a.txt ("one") and b.txt ("bee") committed.
fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    sh(d.path(), &["init", "-q"]);
    sh(d.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(d.path().join("a.txt"), "one\n").unwrap();
    std::fs::write(d.path().join("b.txt"), "bee\n").unwrap();
    sh(d.path(), &["add", "."]);
    sh(d.path(), &["commit", "-q", "-m", "init"]);
    d
}

fn ctx() -> (tempfile::TempDir, DebugCtx) {
    let ws = tempfile::tempdir().unwrap();
    let c = DebugCtx::new(ws.path());
    (ws, c)
}

fn p(d: &tempfile::TempDir) -> &str {
    d.path().to_str().unwrap()
}

fn write(d: &tempfile::TempDir, name: &str, body: &str) {
    std::fs::write(d.path().join(name), body).unwrap();
}

fn read(d: &tempfile::TempDir, name: &str) -> String {
    std::fs::read_to_string(d.path().join(name)).unwrap()
}

// ── run_check ────────────────────────────────────────────────────────────

fn argv(a: &[&str]) -> Option<Vec<String>> {
    Some(a.iter().map(|s| s.to_string()).collect())
}

#[tokio::test]
async fn run_check_rejects_non_allowlisted_and_dangerous_commands() {
    let d = repo();
    let (_ws, c) = ctx();
    for bad in [
        &["sh", "-c", "echo hi"][..],
        &["rm", "-rf", "x"],
        &["/usr/bin/git", "status"],
        &["git", "reset", "--hard"],
        &["git", "clean", "-fd"],
        &["git", "checkout", "main"],
        &["git", "diff", "--output=/tmp/x"],
        &["npm", "publish"],
        &["cargo", "publish"],
        &[],
    ] {
        let e = run_check(&c, Some(p(&d)), None, argv(bad), None, None)
            .await
            .unwrap_err();
        assert!(!e.is_empty(), "{bad:?} should be rejected");
    }
    let e = run_check(&c, Some(p(&d)), None, argv(&["sh"]), None, None)
        .await
        .unwrap_err();
    assert!(e.contains("allowlist"));
    assert!(run_check(&c, Some(p(&d)), Some("nope"), None, None, None)
        .await
        .is_err());
    assert!(run_check(&c, Some(p(&d)), None, None, None, None)
        .await
        .is_err());
    assert!(run_check(
        &c,
        Some(p(&d)),
        Some("x"),
        argv(&["git", "status"]),
        None,
        None
    )
    .await
    .is_err());
    assert!(!d.path().join("x").exists());
}

#[tokio::test]
async fn run_check_runs_allowlisted_argv_and_records_on_the_task() {
    let d = repo();
    let (_ws, c) = ctx();
    let t = task_start(&c, "validate").await.unwrap().value;
    let ok = run_check(
        &c,
        Some(p(&d)),
        None,
        argv(&["git", "rev-parse", "HEAD"]),
        Some(30),
        Some(&t.id),
    )
    .await
    .unwrap()
    .value;
    assert!(ok.passed && ok.exit_code == Some(0) && !ok.timed_out);
    assert_eq!(ok.stdout_tail.trim().len(), 40);
    let bad = run_check(
        &c,
        Some(p(&d)),
        None,
        argv(&["git", "rev-parse", "--verify", "nope"]),
        None,
        Some(&t.id),
    )
    .await
    .unwrap()
    .value;
    assert!(!bad.passed && bad.exit_code != Some(0));
    let rec = task_get(&c, &t.id).await.unwrap().value;
    assert_eq!(rec.validation.len(), 2);
    assert!(rec.validation[0].passed && !rec.validation[1].passed);
    assert!(run_check(
        &c,
        Some(p(&d)),
        None,
        argv(&["git", "status"]),
        None,
        Some("task-nope")
    )
    .await
    .is_err());
}

#[tokio::test]
async fn exec_enforces_timeout_and_kills_the_process_group() {
    let started = std::time::Instant::now();
    let out = exec::run(RunSpec {
        program: "sh",
        args: &["-c".to_string(), "sleep 30 & sleep 30".to_string()],
        cwd: Path::new("/"),
        timeout: Duration::from_millis(300),
        cap: 1024,
        keep: Keep::Tail,
        env: &[],
    })
    .await
    .unwrap();
    assert!(out.timed_out && out.exit_code.is_none() && !out.success());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "timeout must not wait for the child"
    );
}

#[tokio::test]
async fn exec_caps_output_keeping_the_tail() {
    let out = exec::run(RunSpec {
        program: "sh",
        args: &["-c".to_string(), "seq 1 50000".to_string()],
        cwd: Path::new("/"),
        timeout: Duration::from_secs(30),
        cap: 1000,
        keep: Keep::Tail,
        env: &[],
    })
    .await
    .unwrap();
    assert!(out.success() && out.stdout_truncated && out.stdout.len() <= 1000);
    assert!(String::from_utf8_lossy(&out.stdout)
        .trim_end()
        .ends_with("50000"));
    assert!(exec::run(RunSpec {
        program: "definitely-not-a-program",
        args: &[],
        cwd: Path::new("/"),
        timeout: Duration::from_secs(1),
        cap: 10,
        keep: Keep::Head,
        env: &[],
    })
    .await
    .is_err());
}

// ── audit ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn every_op_is_audited_without_contents_or_output() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", "SECRET-CONTENT\n");
    status(&c, Some(p(&d))).await.unwrap();
    let cp = checkpoint_create(&c, Some(p(&d)), "snap", None)
        .await
        .unwrap()
        .value;
    diff(&c, Some(p(&d)), None).await.unwrap();
    run_check(&c, Some(p(&d)), None, argv(&["git", "diff"]), None, None)
        .await
        .unwrap();
    let _ = rollback(&c, Some(p(&d)), &cp.id, false).await;
    task_start(&c, "t").await.unwrap();
    discover_checks(&c, Some(p(&d))).await.unwrap();

    let tail = audit_tail(&c, Some(100)).await.unwrap().value;
    let ops: Vec<&str> = tail.iter().map(|e| e.op.as_str()).collect();
    for op in [
        "status",
        "checkpoint_create",
        "diff",
        "run_check",
        "rollback",
        "task_start",
        "discover_checks",
    ] {
        assert!(ops.contains(&op), "missing {op} in {ops:?}");
    }
    let rb = tail.iter().find(|e| e.op == "rollback").unwrap();
    assert!(rb.outcome.starts_with("error"));
    let raw = serde_json::to_string(&tail).unwrap();
    assert!(!raw.contains("SECRET-CONTENT"));
    assert_eq!(audit_tail(&c, Some(2)).await.unwrap().value.len(), 2);
}

#[test]
fn command_policy_is_an_allowlist_on_subcommands() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("package.json"),
        r#"{"scripts":{"lint":"x","build":"x","test":"x"}}"#,
    )
    .unwrap();
    let check = |a: &[&str]| {
        let v: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        checks::validate_command(d.path(), &v, &[])
    };
    for ok in [
        &["pnpm", "run", "lint"][..],
        &["pnpm", "test"],
        &["pnpm", "lint"],
        &["npm", "run", "build", "--", "--watch=false"],
        &["yarn", "run-script", "lint"],
        &["cargo", "check"],
        &["cargo", "test", "--", "--nocapture"],
        &["cargo", "fmt", "--check"],
        &["cargo", "clippy", "--all-targets"],
        &["git", "diff", "HEAD"],
        &["git", "status", "--porcelain"],
    ] {
        assert!(
            check(ok).is_ok(),
            "{ok:?} should be accepted: {:?}",
            check(ok)
        );
    }
    for bad in [
        &["pnpm", "dlx", "x"][..],
        &["npm", "exec", "x"],
        &["yarn", "dlx", "x"],
        &["cargo", "install", "x"],
        &["cargo", "run"],
        &["npm", "install", "pkg"],
        &["pnpm", "add", "pkg"],
        &["npm", "--prefix", "/tmp", "exec", "x"],
        &["npm", "--prefix", "/tmp", "run", "lint"],
        &["pnpm", "run", "nonexistent"],
        &["pnpm", "run", "--help"],
        &["pnpm", "typecheck"],
        &["npm", "run", "lint", "--prefix", "/tmp"],
        &["npm", "publish"],
        &["cargo", "fmt"],
        &["cargo", "check", "--manifest-path", "/tmp/x/Cargo.toml"],
        &["cargo", "check", "--manifest-path=/tmp/x/Cargo.toml"],
        &["cargo", "check", "--config", "build.rustc-wrapper='sh'"],
        &["cargo", "check", "-Zunstable-options"],
        &["cargo", "--config", "x=y", "check"],
        &["git", "-c", "core.pager=sh", "log"],
        &["git", "diff", "--no-index", "/a", "/b"],
        &["git", "diff", "--output=/tmp/x"],
        &["git", "diff", "--out=/tmp/x"],
        &["git", "grep", "-O", "x"],
        &["git", "diff", "-Osh"],
        &["git", "log", "--exec-path=/x"],
        &["git", "reset", "--hard"],
        &["cargo"],
        &["sh", "-c", "id"],
        &["git", "diff", "/etc/hosts", "/dev/null"],
        &["git", "diff", "~/.ssh/config", "x"],
        &["git", "diff", "../outside", "x"],
        &["git", "show", "HEAD:../secret"],
        &["git", "diff", "--src-prefix=/etc/"],
        &["git", "-C", "/", "status"],
        &["git", "status", "-C/tmp"],
        &["cargo", "clippy", "--fix", "--allow-dirty"],
        &["cargo", "build", "--target-dir", "/tmp/x"],
        &["cargo", "build", "--target-dir=/tmp/x"],
        &["cargo", "build", "--out-dir", "x"],
        &["cargo", "build", "--artifact-dir=x"],
        &["cargo", "check", "--allow-staged"],
    ] {
        assert!(check(bad).is_err(), "{bad:?} should be rejected");
    }
}

fn pid_alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn gone_within(pid: i32, secs: u64) -> bool {
    for _ in 0..(secs * 20) {
        if !pid_alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn spawn_spec<'a>(dir: &'a Path, script: &'a [String], timeout: Duration) -> RunSpec<'a> {
    RunSpec {
        program: "sh",
        args: script,
        cwd: dir,
        timeout,
        cap: 1024,
        keep: Keep::Tail,
        env: &[],
    }
}

fn grandchild_pid(dir: &Path) -> i32 {
    std::fs::read_to_string(dir.join("pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn exec_kills_grandchildren_on_timeout_drop_and_normal_exit() {
    let script = vec![
        "-c".to_string(),
        "sleep 60 & echo $! > pid; sleep 60".to_string(),
    ];
    // Timeout path.
    let d = tempfile::tempdir().unwrap();
    let out = exec::run(spawn_spec(d.path(), &script, Duration::from_millis(500)))
        .await
        .unwrap();
    assert!(out.timed_out);
    assert!(
        gone_within(grandchild_pid(d.path()), 5).await,
        "grandchild survived timeout"
    );

    // Future dropped mid-run (e.g. RPC cancelled).
    let d = tempfile::tempdir().unwrap();
    let dropped = tokio::time::timeout(
        Duration::from_millis(500),
        exec::run(spawn_spec(d.path(), &script, Duration::from_secs(60))),
    )
    .await;
    assert!(dropped.is_err());
    assert!(
        gone_within(grandchild_pid(d.path()), 5).await,
        "grandchild survived drop"
    );

    // Normal exit leaving a background process behind.
    let d = tempfile::tempdir().unwrap();
    let script = vec![
        "-c".to_string(),
        "sleep 60 & echo $! > pid; exit 0".to_string(),
    ];
    let out = exec::run(spawn_spec(d.path(), &script, Duration::from_secs(30)))
        .await
        .unwrap();
    assert!(out.success());
    assert!(
        gone_within(grandchild_pid(d.path()), 5).await,
        "leftover background process"
    );
}

#[tokio::test]
async fn exec_scrubs_inherited_git_environment() {
    std::env::set_var("GIT_CEILING_DIRECTORIES", "/nonexistent-neppy-test");
    let script = vec![
        "-c".to_string(),
        "printf %s \"${GIT_CEILING_DIRECTORIES-unset}\"".to_string(),
    ];
    let out = exec::run(spawn_spec(Path::new("/"), &script, Duration::from_secs(10)))
        .await
        .unwrap();
    std::env::remove_var("GIT_CEILING_DIRECTORIES");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "unset");
}

#[tokio::test]
async fn rollback_removes_directories_it_empties_but_not_others() {
    let d = repo();
    let (_ws, c) = ctx();
    std::fs::create_dir(d.path().join("keepdir")).unwrap();
    let cp = checkpoint_create(&c, Some(p(&d)), "base", None)
        .await
        .unwrap()
        .value;
    std::fs::create_dir_all(d.path().join("new/deep")).unwrap();
    write(&d, "new/deep/f.txt", "f\n");
    std::fs::create_dir(d.path().join("mixed")).unwrap();
    write(&d, "mixed/g.txt", "g\n");
    rollback(&c, Some(p(&d)), &cp.id, true).await.unwrap();
    assert!(!d.path().join("new").exists(), "emptied dirs are removed");
    assert!(!d.path().join("mixed").exists());
    assert!(
        d.path().join("keepdir").exists(),
        "pre-existing empty dir untouched"
    );
    assert!(d.path().exists());
}

#[tokio::test]
async fn rollback_failure_after_pre_checkpoint_reports_its_id() {
    let d = repo();
    let (_ws, c) = ctx();
    let mut cp = checkpoint_create(&c, Some(p(&d)), "base", None)
        .await
        .unwrap()
        .value;
    cp.index_tree = "a".repeat(40); // read-tree will fail
    let git = crate::neppy::agent::debug_mode::git::Git::new(Path::new(&cp.project_root));
    let err = crate::neppy::agent::debug_mode::checkpoints::rollback(&c.store, &git, &cp)
        .await
        .unwrap_err();
    let pre = &checkpoint_list(&c, Some(1)).await.unwrap().value[0];
    assert!(
        err.contains(&format!("pre_rollback_checkpoint_id={}", pre.id)),
        "{err}"
    );
}
