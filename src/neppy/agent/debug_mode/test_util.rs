//! Shared fixtures for the Debug Mode turn/tool tests.

use std::path::Path;

fn sh(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}");
}

/// Temp repo on `main` with `a.txt` ("one") committed.
pub(super) fn repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    sh(d.path(), &["init", "-q"]);
    sh(d.path(), &["symbolic-ref", "HEAD", "refs/heads/main"]);
    std::fs::write(d.path().join("a.txt"), "one\n").unwrap();
    sh(d.path(), &["add", "."]);
    sh(d.path(), &["commit", "-q", "-m", "init"]);
    d
}
