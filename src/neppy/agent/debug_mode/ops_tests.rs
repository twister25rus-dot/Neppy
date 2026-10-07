use std::path::Path;

use super::*;
use crate::neppy::agent::debug_mode::git::{parse_porcelain_z, resolve_project_root};

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

// ── status ───────────────────────────────────────────────────────────────

#[test]
fn porcelain_is_parsed_into_buckets() {
    let d = parse_porcelain_z(
        b" M a.txt\0A  new.txt\0 D gone.txt\0?? u.txt\0R  moved.txt\0old.txt\0MM both.txt\0",
    );
    assert_eq!(d.modified, ["a.txt", "both.txt", "moved.txt"]);
    assert_eq!(d.added, ["new.txt"]);
    assert_eq!(d.deleted, ["gone.txt"]);
    assert_eq!(d.untracked, ["u.txt"]);
}

#[tokio::test]
async fn status_reports_branch_head_dirty_and_active_task() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", "two\n");
    write(&d, "new.txt", "n\n");
    sh(d.path(), &["add", "new.txt"]);
    std::fs::remove_file(d.path().join("b.txt")).unwrap();
    write(&d, "u.txt", "u\n");

    let s = status(&c, Some(p(&d))).await.unwrap().value;
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert_eq!(s.head.as_deref().map(str::len), Some(40));
    assert_eq!(s.dirty.modified, ["a.txt"]);
    assert_eq!(s.dirty.added, ["new.txt"]);
    assert_eq!(s.dirty.deleted, ["b.txt"]);
    assert_eq!(s.dirty.untracked, ["u.txt"]);
    assert!(!s.task_active);

    let t = task_start(&c, "fix it").await.unwrap().value;
    let s = status(&c, Some(p(&d))).await.unwrap().value;
    assert!(s.task_active);
    assert_eq!(s.active_task_id.as_deref(), Some(t.id.as_str()));
}

#[tokio::test]
async fn project_root_must_be_a_git_worktree_root() {
    let plain = tempfile::tempdir().unwrap();
    assert!(resolve_project_root(Some(p(&plain)))
        .await
        .unwrap_err()
        .contains("git work tree"));
    let d = repo();
    std::fs::create_dir(d.path().join("sub")).unwrap();
    let sub = d.path().join("sub");
    assert!(resolve_project_root(sub.to_str())
        .await
        .unwrap_err()
        .contains("not the root"));
    assert!(resolve_project_root(Some("/definitely/not/here"))
        .await
        .is_err());
    let ok = resolve_project_root(Some(p(&d))).await.unwrap();
    assert_eq!(ok, std::fs::canonicalize(d.path()).unwrap());
}

// ── discovery ────────────────────────────────────────────────────────────

#[test]
fn checks_are_derived_from_project_files() {
    let d = tempfile::tempdir().unwrap();
    write(
        &d,
        "package.json",
        r#"{"scripts":{"lint":"x","typecheck":"x","test":"echo \"Error: no test specified\" && exit 1","build":"x","format":"prettier --write .","format:check":"x","debug:check":"x"}}"#,
    );
    write(&d, "pnpm-lock.yaml", "");
    write(&d, "Cargo.toml", "[package]\nname=\"x\"\n");
    let found = checks::discover(d.path());
    assert!(found[0].preferred);
    assert_eq!(found[0].command, ["pnpm", "run", "debug:check"]);
    let ids: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
    assert!(
        ids.contains(&"pnpm:lint")
            && ids.contains(&"pnpm:typecheck")
            && ids.contains(&"pnpm:build")
    );
    assert!(ids.contains(&"pnpm:format:check"));
    assert!(
        !ids.contains(&"pnpm:test"),
        "npm init placeholder is not a test"
    );
    assert!(
        !ids.contains(&"pnpm:format"),
        "mutating format script is never offered"
    );
    assert!(ids.contains(&"cargo:check") && ids.contains(&"cargo:test"));
    let lint = found.iter().find(|c| c.id == "pnpm:lint").unwrap();
    assert_eq!(lint.kind, CheckKind::Lint);
}

#[test]
fn package_manager_follows_the_lockfile_and_defaults_to_npm() {
    let d = tempfile::tempdir().unwrap();
    write(&d, "package.json", r#"{"scripts":{"build":"x"}}"#);
    assert_eq!(
        checks::discover(d.path())[0].command,
        ["npm", "run", "build"]
    );
    write(&d, "yarn.lock", "");
    assert_eq!(
        checks::discover(d.path())[0].command,
        ["yarn", "run", "build"]
    );
    let empty = tempfile::tempdir().unwrap();
    assert!(checks::discover(empty.path()).is_empty());
}

// ── checkpoints ──────────────────────────────────────────────────────────

fn snapshot(d: &tempfile::TempDir) -> (String, String, String, String, String) {
    (
        sh(
            d.path(),
            &["status", "--porcelain=v1", "--untracked-files=all"],
        ),
        sh(d.path(), &["diff", "--cached"]),
        sh(d.path(), &["diff"]),
        sh(d.path(), &["symbolic-ref", "HEAD"]),
        sh(d.path(), &["rev-parse", "HEAD"]),
    )
}

#[tokio::test]
async fn checkpoint_create_does_not_touch_tree_index_or_branch() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", "two\n");
    write(&d, "staged.txt", "s\n");
    sh(d.path(), &["add", "staged.txt"]);
    write(&d, "u.txt", "u\n");
    let before = snapshot(&d);

    let cp = checkpoint_create(&c, Some(p(&d)), "before refactor", None)
        .await
        .unwrap()
        .value;
    assert_eq!(snapshot(&d), before);
    assert_eq!(read(&d, "a.txt"), "two\n");
    assert_ne!(
        cp.snapshot_sha, cp.head,
        "dirty tree is captured in its own commit"
    );
    assert_eq!(cp.untracked_files, ["u.txt"]);
    assert!(cp.dirty_files.contains(&"a.txt".to_string()));
    // Pinned so GC cannot collect it.
    let pinned = sh(
        d.path(),
        &["rev-parse", &format!("{CHECKPOINT_REF_PREFIX}{}", cp.id)],
    );
    assert_eq!(pinned.trim(), cp.snapshot_sha);
    assert_eq!(checkpoint_get(&c, &cp.id).await.unwrap().value.id, cp.id);
    assert_eq!(checkpoint_list(&c, None).await.unwrap().value.len(), 1);
}

#[tokio::test]
async fn clean_tree_checkpoint_falls_back_to_head() {
    let d = repo();
    let (_ws, c) = ctx();
    let cp = checkpoint_create(&c, Some(p(&d)), "clean", None)
        .await
        .unwrap()
        .value;
    assert_eq!(cp.snapshot_sha, cp.head);
    assert!(checkpoint_create(&c, Some(p(&d)), "  ", None)
        .await
        .is_err());
}

#[tokio::test]
async fn rollback_refuses_without_confirm() {
    let d = repo();
    let (_ws, c) = ctx();
    let cp = checkpoint_create(&c, Some(p(&d)), "x", None)
        .await
        .unwrap()
        .value;
    write(&d, "a.txt", "changed\n");
    let err = rollback(&c, Some(p(&d)), &cp.id, false).await.unwrap_err();
    assert!(err.contains("confirm"));
    assert_eq!(read(&d, "a.txt"), "changed\n");
    // Refusal did not even create a pre-rollback checkpoint.
    assert_eq!(checkpoint_list(&c, None).await.unwrap().value.len(), 1);
    assert!(rollback(&c, Some(p(&d)), "cp-nope", true)
        .await
        .unwrap_err()
        .contains("unknown"));
}

#[tokio::test]
async fn rollback_restores_tracked_removes_new_keeps_preexisting_untracked() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", "two\n");
    write(&d, "keep.txt", "mine\n"); // untracked before the checkpoint
    let t = task_start(&c, "work").await.unwrap().value;
    let cp = checkpoint_create(&c, Some(p(&d)), "good state", Some(&t.id))
        .await
        .unwrap()
        .value;

    write(&d, "a.txt", "three\n");
    std::fs::remove_file(d.path().join("b.txt")).unwrap();
    write(&d, "new.txt", "n\n"); // created after the checkpoint
    write(&d, "staged_new.txt", "s\n");
    sh(d.path(), &["add", "staged_new.txt"]);
    write(&d, "keep.txt", "edited later\n");
    let head_before = sh(d.path(), &["rev-parse", "HEAD"]);

    let r = rollback(&c, Some(p(&d)), &cp.id, true).await.unwrap().value;
    assert_eq!(read(&d, "a.txt"), "two\n");
    assert_eq!(read(&d, "b.txt"), "bee\n");
    assert!(!d.path().join("new.txt").exists());
    assert!(!d.path().join("staged_new.txt").exists());
    assert_eq!(
        read(&d, "keep.txt"),
        "mine\n",
        "pre-existing untracked file is kept (and reverted to its checkpoint content)"
    );
    assert_eq!(r.removed, ["new.txt", "staged_new.txt"]);
    assert!(r.restored.contains(&"a.txt".to_string()) && r.restored.contains(&"b.txt".to_string()));
    assert!(!r.head_moved);
    assert_eq!(sh(d.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(
        sh(d.path(), &["symbolic-ref", "--short", "HEAD"]).trim(),
        "main"
    );
    assert_eq!(
        task_get(&c, &t.id).await.unwrap().value.status,
        TaskStatus::RolledBack
    );

    // Rollback is itself reversible via its automatic checkpoint.
    let pre = checkpoint_get(&c, &r.pre_rollback_checkpoint_id)
        .await
        .unwrap()
        .value;
    assert!(pre.description.starts_with("pre-rollback of "));
    rollback(&c, Some(p(&d)), &pre.id, true).await.unwrap();
    assert_eq!(read(&d, "a.txt"), "three\n");
    assert!(!d.path().join("b.txt").exists());
    assert_eq!(
        read(&d, "new.txt"),
        "n\n",
        "files deleted by rollback come back"
    );
}

#[tokio::test]
async fn rolling_back_a_rollback_restores_deleted_untracked_files() {
    let d = repo();
    let (_ws, c) = ctx();
    let cp = checkpoint_create(&c, Some(p(&d)), "base", None)
        .await
        .unwrap()
        .value;
    write(&d, "made_later.txt", "original contents\n");
    let r = rollback(&c, Some(p(&d)), &cp.id, true).await.unwrap().value;
    assert!(!d.path().join("made_later.txt").exists());
    assert_eq!(r.removed, ["made_later.txt"]);
    rollback(&c, Some(p(&d)), &r.pre_rollback_checkpoint_id, true)
        .await
        .unwrap();
    assert_eq!(read(&d, "made_later.txt"), "original contents\n");
}

#[tokio::test]
async fn staged_versus_unstaged_split_survives_rollback() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", "staged\n");
    sh(d.path(), &["add", "a.txt"]);
    write(&d, "a.txt", "unstaged\n");
    let cp = checkpoint_create(&c, Some(p(&d)), "split", None)
        .await
        .unwrap()
        .value;
    assert!(!cp.index_tree.is_empty());
    let pinned = sh(
        d.path(),
        &[
            "rev-parse",
            &format!("{CHECKPOINT_REF_PREFIX}{}-index", cp.id),
        ],
    );
    assert_eq!(pinned.trim(), cp.index_tree);

    write(&d, "a.txt", "other\n");
    sh(d.path(), &["add", "a.txt"]);
    rollback(&c, Some(p(&d)), &cp.id, true).await.unwrap();
    assert_eq!(read(&d, "a.txt"), "unstaged\n");
    assert_eq!(sh(d.path(), &["show", ":a.txt"]), "staged\n");
}

#[tokio::test]
async fn ignored_files_are_untouched_by_checkpoint_and_rollback() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, ".gitignore", ".env\n");
    sh(d.path(), &["add", ".gitignore"]);
    sh(d.path(), &["commit", "-q", "-m", "ignore"]);
    write(&d, ".env", "TOKEN=1\n");
    let cp = checkpoint_create(&c, Some(p(&d)), "with env", None)
        .await
        .unwrap()
        .value;
    let files = sh(
        d.path(),
        &["ls-tree", "-r", "--name-only", &cp.snapshot_sha],
    );
    assert!(!files.contains(".env"), "ignored file must not be captured");

    write(&d, ".env", "TOKEN=2\n");
    write(&d, "x.txt", "x\n");
    let r = rollback(&c, Some(p(&d)), &cp.id, true).await.unwrap().value;
    assert_eq!(read(&d, ".env"), "TOKEN=2\n");
    assert_eq!(r.removed, ["x.txt"]);
    // And an ignored file created after the checkpoint is not deleted either.
    write(&d, ".env", "TOKEN=3\n");
    rollback(&c, Some(p(&d)), &cp.id, true).await.unwrap();
    assert_eq!(read(&d, ".env"), "TOKEN=3\n");
}

// ── diff ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn diff_summarises_against_head_and_a_checkpoint() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", "two\n");
    std::fs::remove_file(d.path().join("b.txt")).unwrap();
    write(&d, "u.txt", "u\n");
    let r = diff(&c, Some(p(&d)), None).await.unwrap().value;
    assert_eq!(r.base, "HEAD");
    assert!(r.text.contains("+two") && !r.truncated);
    assert!(
        r.text.contains("u.txt"),
        "new files appear in the diff text"
    );
    assert_eq!(
        (r.summary.modified, r.summary.created, r.summary.deleted),
        (1, 1, 1)
    );
    assert_eq!(r.untracked, ["u.txt"]);
    assert!(r
        .files
        .iter()
        .any(|f| f.path == "a.txt" && f.added == Some(1) && f.deleted == Some(1)));

    let cp = checkpoint_create(&c, Some(p(&d)), "mid", None)
        .await
        .unwrap()
        .value;
    write(&d, "a.txt", "three\n");
    let r = diff(&c, Some(p(&d)), Some(&cp.id)).await.unwrap().value;
    assert_eq!(r.base, cp.id);
    assert!(r.text.contains("+three") && r.text.contains("-two"));
    assert_eq!(
        (r.summary.modified, r.summary.created, r.summary.deleted),
        (1, 0, 0)
    );
    assert!(diff(&c, Some(p(&d)), Some("cp-nope")).await.is_err());
}

#[tokio::test]
async fn diff_text_is_capped_and_flagged() {
    let d = repo();
    let (_ws, c) = ctx();
    write(&d, "a.txt", &"x".repeat(300 * 1024));
    let r = diff(&c, Some(p(&d)), None).await.unwrap().value;
    assert!(r.truncated);
    assert!(r.text.len() <= DIFF_CAP);
}

// ── tasks ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn task_lifecycle_and_snake_case_status() {
    let (ws, c) = ctx();
    let t = task_start(&c, "add streaming").await.unwrap().value;
    assert_eq!(t.status, TaskStatus::Planning);
    assert!(task_start(&c, "  ").await.is_err());

    let patch: TaskPatch = serde_json::from_value(serde_json::json!({
        "status": "rolled_back", "files_changed": ["a.rs"], "summary": "undone", "branch": "debug/x"
    }))
    .unwrap();
    let u = task_update(&c, &t.id, patch).await.unwrap().value;
    assert_eq!(serde_json::to_value(u.status).unwrap(), "rolled_back");
    assert_eq!(
        (u.files_changed.clone(), u.branch.as_deref()),
        (vec!["a.rs".to_string()], Some("debug/x"))
    );
    assert!(
        serde_json::from_value::<TaskPatch>(serde_json::json!({"status": "ROLLED_BACK"})).is_err()
    );
    assert!(serde_json::from_value::<TaskPatch>(serde_json::json!({"bogus": 1})).is_err());
    assert!(task_update(&c, "task-nope", TaskPatch::default())
        .await
        .is_err());
    let bad = TaskPatch {
        checkpoint_id: Some("cp-nope".into()),
        ..Default::default()
    };
    assert!(task_update(&c, &t.id, bad)
        .await
        .unwrap_err()
        .contains("unknown checkpoint"));

    // Survives a restart.
    let c2 = DebugCtx::new(ws.path());
    assert_eq!(task_list(&c2, None).await.unwrap().value[0].id, t.id);
    assert_eq!(
        task_get(&c2, &t.id).await.unwrap().value.summary.as_deref(),
        Some("undone")
    );
}
