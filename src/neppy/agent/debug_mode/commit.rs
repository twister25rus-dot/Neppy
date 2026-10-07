//! `debug_mode_commit`: commits exactly the files a debug task changed.
//!
//! Safety properties, each pinned by a test in `commit_tests.rs`:
//! - refuses unless `confirm=true`;
//! - stages only the task's recorded `files_changed` (validated as relative,
//!   `..`-free, outside `.git`, and matched literally — never as a glob or
//!   pathspec magic);
//! - refuses when the index already holds staged changes outside those paths,
//!   so unrelated user work is never swept into the commit;
//! - never uses `--no-verify` or `--amend`, and never pushes.

use std::collections::BTreeSet;
use std::path::{Component, Path};
use std::time::Duration;

use super::git::{split_nul, Git};
use super::ops::{audit, clip, done, git_for, DebugCtx, RpcResult, REPO_LOCK};
use super::types::{CommitResult, TaskRecord, TaskStatus};

/// Pre-commit hooks may run a full lint; give them room (git itself is 60 s).
const COMMIT_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_MESSAGE: usize = 4000;
const LITERAL: [(&str, &str); 1] = [("GIT_LITERAL_PATHSPECS", "1")];

/// A task-recorded path must be a plain relative path inside the work tree.
fn validate_path(p: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("refusing path '{}': {why}", clip(p, 200)));
    if p.is_empty() || p.contains('\0') {
        return bad("empty or contains NUL");
    }
    let path = Path::new(p);
    if path.is_absolute() || p.starts_with('/') || p.starts_with('\\') {
        return bad("absolute path");
    }
    let mut first = true;
    for c in path.components() {
        match c {
            Component::Normal(seg) => {
                if first && seg == ".git" {
                    return bad("inside .git");
                }
                first = false;
            }
            _ => return bad("must be a plain relative path without '..'"),
        }
    }
    if first {
        return bad("no path component");
    }
    Ok(())
}

async fn staged_paths(git: &Git) -> Result<BTreeSet<String>, String> {
    let out = git
        .ok(&["diff", "--cached", "--name-only", "--no-renames", "-z"])
        .await?;
    Ok(split_nul(&out).into_iter().collect())
}

fn outside(staged: &BTreeSet<String>, allowed: &BTreeSet<String>) -> Vec<String> {
    staged.difference(allowed).cloned().collect()
}

fn unrelated_error(extra: &[String]) -> String {
    let shown: Vec<&str> = extra.iter().take(20).map(String::as_str).collect();
    let more = extra.len().saturating_sub(shown.len());
    format!(
        "commit refused: the index already has staged changes outside this task's files: {}{}. \
         Unstage or commit them first so unrelated work is not included.",
        shown.join(", "),
        if more > 0 {
            format!(" (and {more} more)")
        } else {
            String::new()
        }
    )
}

async fn is_in_index(git: &Git, path: &str) -> Result<bool, String> {
    let out = git
        .ok_env(&["ls-files", "--cached", "-z", "--", path], &LITERAL)
        .await?;
    Ok(!out.is_empty())
}

async fn stage(git: &Git, paths: &[String]) -> Result<(), String> {
    for p in paths {
        let on_disk = std::fs::symlink_metadata(git.root().join(p)).is_ok();
        if on_disk {
            git.ok_env(&["add", "--", p], &LITERAL).await?;
        } else if is_in_index(git, p).await? {
            // Deleted in the work tree: record the deletion.
            git.ok_env(&["add", "-A", "--", p], &LITERAL).await?;
        } else {
            log::debug!("[debug_mode] commit skip path (absent and untracked) path={p}");
        }
    }
    Ok(())
}

async fn precheck(
    ctx: &DebugCtx,
    task_id: &str,
    message: &str,
    confirm: bool,
) -> Result<TaskRecord, String> {
    super::policy::ensure_commit_allowed(&ctx.settings)?;
    if !confirm {
        return Err("commit refused: pass confirm=true to proceed".to_string());
    }
    if message.trim().is_empty() {
        return Err("commit message must not be empty".to_string());
    }
    if message.contains('\0') {
        return Err("commit message must not contain NUL".to_string());
    }
    let task = ctx
        .store
        .task_get(task_id)?
        .ok_or_else(|| format!("unknown task '{task_id}'"))?;
    if task.status.is_active() {
        return Err("commit refused: the task is still running".to_string());
    }
    if task.status == TaskStatus::RolledBack {
        return Err("commit refused: the task was rolled back".to_string());
    }
    if let Some(c) = &task.commit {
        return Err(format!("commit refused: task already committed as {c}"));
    }
    if task.files_changed.is_empty() {
        return Err("commit refused: the task recorded no changed files".to_string());
    }
    for p in &task.files_changed {
        validate_path(p)?;
    }
    Ok(task)
}

pub async fn commit(
    ctx: &DebugCtx,
    project_root: Option<&str>,
    task_id: &str,
    message: &str,
    confirm: bool,
) -> RpcResult<CommitResult> {
    let r = async {
        let task = precheck(ctx, task_id, message, confirm).await?;
        let cp_root = match &task.checkpoint_id {
            Some(id) => ctx.store.checkpoint_get(id)?.map(|c| c.project_root),
            None => None,
        };
        let git = git_for(ctx, project_root.or(cp_root.as_deref())).await?;
        let allowed: BTreeSet<String> = task.files_changed.iter().cloned().collect();
        let paths: Vec<String> = allowed.iter().cloned().collect();

        let _g = REPO_LOCK.lock().await;
        let extra = outside(&staged_paths(&git).await?, &allowed);
        if !extra.is_empty() {
            return Err(unrelated_error(&extra));
        }
        stage(&git, &paths).await?;
        // Re-check after staging: nothing outside the task may be in the index.
        let staged = staged_paths(&git).await?;
        let extra = outside(&staged, &allowed);
        if !extra.is_empty() {
            return Err(unrelated_error(&extra));
        }
        if staged.is_empty() {
            return Err("commit refused: none of the task's files have changes to commit".into());
        }
        log::debug!(
            "[debug_mode] commit task={task_id} files={} message_len={}",
            staged.len(),
            message.len()
        );
        let out = git
            .run(&["commit", "-m", message], Some(COMMIT_TIMEOUT))
            .await?;
        if out.timed_out {
            return Err("git commit timed out (files remain staged)".to_string());
        }
        if out.exit_code != Some(0) {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stderr),
                String::from_utf8_lossy(&out.stdout)
            );
            let detail: Vec<&str> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .take(8)
                .collect();
            return Err(format!(
                "git commit failed (exit {:?}; the task's files remain staged): {}",
                out.exit_code,
                clip(&detail.join(" | "), 800)
            ));
        }
        let sha = git
            .head()
            .await?
            .ok_or_else(|| "commit succeeded but HEAD could not be read".to_string())?;
        let branch = git.branch().await?;
        let (sha2, branch2) = (sha.clone(), branch.clone());
        if let Err(e) = ctx.store.task_modify(task_id, move |t| {
            t.commit = Some(sha2);
            if branch2.is_some() {
                t.branch = branch2;
            }
        }) {
            // The commit exists; a history hiccup must not report failure.
            log::warn!("[debug_mode] commit done but task update failed: {e}");
        }
        Ok(CommitResult {
            task_id: task_id.to_string(),
            commit: sha,
            branch,
            files: staged.into_iter().collect(),
        })
    }
    .await;
    let target = match &r {
        Ok(c) => format!("{task_id} {}", c.commit),
        Err(_) => task_id.to_string(),
    };
    audit(ctx, "commit", &target, &r);
    r.and_then(done)
}

#[cfg(test)]
#[path = "commit_tests.rs"]
mod commit_tests;
