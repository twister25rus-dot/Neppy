use std::path::Path;

use super::*;
use crate::neppy::agent::debug_mode::ops;
use crate::neppy::agent::debug_mode::ops::{task_start, task_update};
use crate::neppy::agent::debug_mode::types::TaskPatch;

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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    sh(d.path(), &["init", "-q"]);
    sh(d.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(d.path().join("a.txt"), "one\n").unwrap();
    std::fs::write(d.path().join("b.txt"), "bee\n").unwrap();
    std::fs::write(d.path().join("gone.txt"), "bye\n").unwrap();
    sh(d.path(), &["add", "."]);
    sh(d.path(), &["commit", "-q", "-m", "init"]);
    d
}

fn p(d: &tempfile::TempDir) -> &str {
    d.path().to_str().unwrap()
}

fn ctx() -> (tempfile::TempDir, DebugCtx) {
    let ws = tempfile::tempdir().unwrap();
    let c = DebugCtx::new(ws.path());
    (ws, c)
}

async fn task_with(c: &DebugCtx, files: &[&str], status: TaskStatus) -> String {
    let t = task_start(c, "fix things").await.unwrap().value;
    task_update(
        c,
        &t.id,
        TaskPatch {
            status: Some(status),
            files_changed: Some(files.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    t.id
}

#[tokio::test]
async fn commits_only_the_task_files_and_records_the_sha() {
    let d = repo();
    let (_ws, c) = ctx();
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    std::fs::write(d.path().join("new.txt"), "new\n").unwrap();
    std::fs::remove_file(d.path().join("gone.txt")).unwrap();
    // Unrelated, unstaged user work must stay uncommitted.
    std::fs::write(d.path().join("b.txt"), "user edit\n").unwrap();
    let id = task_with(&c, &["a.txt", "new.txt", "gone.txt"], TaskStatus::Pass).await;

    let r = commit(&c, Some(p(&d)), &id, "feat(debug): x", true)
        .await
        .unwrap()
        .value;
    assert_eq!(r.files, vec!["a.txt", "gone.txt", "new.txt"]);
    assert_eq!(r.commit, sh(d.path(), &["rev-parse", "HEAD"]).trim());
    let names = sh(d.path(), &["show", "--name-only", "--format=", "HEAD"]);
    let mut got: Vec<&str> = names.lines().collect();
    got.sort();
    assert_eq!(got, vec!["a.txt", "gone.txt", "new.txt"]);
    let status = sh(d.path(), &["status", "--porcelain"]);
    assert_eq!(status.trim(), "M b.txt", "unrelated edit left alone");
    let t = ops::task_get(&c, &id).await.unwrap().value;
    assert_eq!(t.commit.as_deref(), Some(r.commit.as_str()));
    assert_eq!(t.branch.as_deref(), Some("main"));
    let tail = ops::audit_tail(&c, Some(5)).await.unwrap().value;
    assert!(tail.iter().any(|e| e.op == "commit" && e.outcome == "ok"));
    // A second commit of the same task is refused.
    let err = commit(&c, Some(p(&d)), &id, "again", true)
        .await
        .unwrap_err();
    assert!(err.contains("already committed"));
}

#[tokio::test]
async fn refuses_without_confirm_and_changes_nothing() {
    let d = repo();
    let (_ws, c) = ctx();
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    let id = task_with(&c, &["a.txt"], TaskStatus::Pass).await;
    let err = commit(&c, Some(p(&d)), &id, "m", false).await.unwrap_err();
    assert!(err.contains("confirm"));
    assert_eq!(sh(d.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert_eq!(sh(d.path(), &["diff", "--cached", "--name-only"]), "");
}

#[tokio::test]
async fn refuses_when_unrelated_files_are_already_staged() {
    let d = repo();
    let (_ws, c) = ctx();
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    std::fs::write(d.path().join("b.txt"), "user staged\n").unwrap();
    sh(d.path(), &["add", "b.txt"]);
    let id = task_with(&c, &["a.txt"], TaskStatus::Pass).await;
    let err = commit(&c, Some(p(&d)), &id, "m", true).await.unwrap_err();
    assert!(err.contains("b.txt"), "{err}");
    assert!(err.contains("outside this task"));
    assert_eq!(sh(d.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");
    // Nothing of the task was staged either.
    assert_eq!(
        sh(d.path(), &["diff", "--cached", "--name-only"]).trim(),
        "b.txt"
    );
}

#[tokio::test]
async fn refuses_unsafe_paths_and_bad_task_states() {
    let d = repo();
    let (_ws, c) = ctx();
    for bad in ["../x", "/etc/passwd", "a/../../x", ".git/config", ""] {
        let id = task_with(&c, &[bad], TaskStatus::Pass).await;
        assert!(
            commit(&c, Some(p(&d)), &id, "m", true).await.is_err(),
            "{bad:?} must be refused"
        );
    }
    let running = task_with(&c, &["a.txt"], TaskStatus::Editing).await;
    assert!(commit(&c, Some(p(&d)), &running, "m", true)
        .await
        .unwrap_err()
        .contains("still running"));
    let rb = task_with(&c, &["a.txt"], TaskStatus::RolledBack).await;
    assert!(commit(&c, Some(p(&d)), &rb, "m", true)
        .await
        .unwrap_err()
        .contains("rolled back"));
    let none = task_with(&c, &[], TaskStatus::Pass).await;
    assert!(commit(&c, Some(p(&d)), &none, "m", true).await.is_err());
    let ok = task_with(&c, &["a.txt"], TaskStatus::Pass).await;
    assert!(commit(&c, Some(p(&d)), &ok, "  ", true).await.is_err());
    assert!(commit(&c, Some(p(&d)), "task-nope", "m", true)
        .await
        .unwrap_err()
        .contains("unknown task"));
}

#[tokio::test]
async fn refuses_when_task_files_have_no_changes() {
    let d = repo();
    let (_ws, c) = ctx();
    let id = task_with(&c, &["a.txt"], TaskStatus::Pass).await;
    let err = commit(&c, Some(p(&d)), &id, "m", true).await.unwrap_err();
    assert!(err.contains("have changes to commit"), "{err}");
}

#[tokio::test]
async fn failing_hook_is_not_bypassed_and_files_stay_staged() {
    let d = repo();
    let (_ws, c) = ctx();
    let hook = d.path().join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(d.path().join("a.txt"), "changed\n").unwrap();
    let id = task_with(&c, &["a.txt"], TaskStatus::Pass).await;
    let err = commit(&c, Some(p(&d)), &id, "m", true).await.unwrap_err();
    assert!(err.contains("git commit failed"), "{err}");
    assert_eq!(sh(d.path(), &["rev-list", "--count", "HEAD"]).trim(), "1");
    assert!(ops::task_get(&c, &id).await.unwrap().value.commit.is_none());
}

#[test]
fn validate_path_accepts_nested_relative_paths() {
    assert!(validate_path("src/a/b.rs").is_ok());
    assert!(validate_path("./a").is_err());
}
