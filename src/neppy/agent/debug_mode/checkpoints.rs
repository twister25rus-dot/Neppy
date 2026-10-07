//! Checkpoints, rollback and diff — the git-facing half of Debug Mode.
//!
//! A checkpoint is non-destructive and complete. The working tree (tracked
//! files plus untracked, non-ignored ones) is written into a tree object via a
//! *copy* of the index (`GIT_INDEX_FILE`), wrapped in a commit whose parent is
//! HEAD, and pinned under `refs/neppy-debug/checkpoints/<id>`; the staged state
//! is pinned as a tree under `<id>-index`. The real index, the working tree and
//! the branch are never touched. Ignored files are in neither tree.
//!
//! Rollback restores files from that commit, re-reads the staged tree into the
//! index, and deletes only untracked files absent from the snapshot. Because
//! it first takes a checkpoint of the current state (which includes the files
//! about to be deleted), a rollback can itself be rolled back. It never uses
//! `reset --hard`, `clean` or a branch checkout.

use std::collections::HashSet;
use std::path::{Component, Path};

use super::git::{split_nul, Git};
use super::store::DebugStore;
use super::types::*;

pub async fn create(
    store: &DebugStore,
    git: &Git,
    description: &str,
    task_id: Option<&str>,
) -> Result<Checkpoint, String> {
    let head = git
        .head()
        .await?
        .ok_or("repository has no commits yet; a checkpoint needs a HEAD")?;
    let branch = git.branch().await?;
    let dirty = git.dirty().await?;
    let trees = git.snapshot_trees().await?;
    let head_tree = git.ok_str(&["rev-parse", "HEAD^{tree}"]).await?;

    let now = chrono::Utc::now();
    let id = format!(
        "cp-{}-{}",
        now.format("%Y%m%d%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    // A working tree identical to HEAD needs no extra commit.
    let snapshot_sha = if trees.worktree_tree == head_tree {
        head.clone()
    } else {
        git.ok_str(&[
            "commit-tree",
            &trees.worktree_tree,
            "-p",
            &head,
            "-m",
            &format!("neppy debug checkpoint {id}"),
        ])
        .await?
    };
    let ref_name = format!("{CHECKPOINT_REF_PREFIX}{id}");
    let index_ref = format!("{ref_name}-index");
    git.ok(&["update-ref", &ref_name, &snapshot_sha]).await?;
    if let Err(e) = git.ok(&["update-ref", &index_ref, &trees.index_tree]).await {
        let _ = git.ok(&["update-ref", "-d", &ref_name]).await;
        return Err(e);
    }

    let cp = Checkpoint {
        id: id.clone(),
        description: description.chars().take(500).collect(),
        task_id: task_id.map(str::to_string),
        project_root: git.root().display().to_string(),
        created_at: now.to_rfc3339(),
        head,
        branch,
        snapshot_sha,
        index_tree: trees.index_tree,
        dirty_files: dirty.tracked_changes(),
        untracked_files: dirty.untracked,
    };
    if let Err(e) = store.checkpoint_add(cp.clone()) {
        // Do not leave an unreferenced pin behind.
        let _ = git.ok(&["update-ref", "-d", &ref_name]).await;
        let _ = git.ok(&["update-ref", "-d", &index_ref]).await;
        return Err(e);
    }
    log::debug!(
        "[debug_mode] checkpoint created id={id} dirty={} untracked={}",
        cp.dirty_files.len(),
        cp.untracked_files.len()
    );
    Ok(cp)
}

pub async fn rollback(
    store: &DebugStore,
    git: &Git,
    cp: &Checkpoint,
) -> Result<RollbackResult, String> {
    if Path::new(&cp.project_root) != git.root() {
        return Err(format!(
            "checkpoint {} belongs to a different project root",
            cp.id
        ));
    }
    git.ok(&["cat-file", "-e", &format!("{}^{{commit}}", cp.snapshot_sha)])
        .await
        .map_err(|e| format!("checkpoint snapshot is missing: {e}"))?;

    // Make the rollback itself reversible before touching anything.
    let pre = create(
        store,
        git,
        &format!("pre-rollback of {}", cp.id),
        cp.task_id.as_deref(),
    )
    .await?;

    rollback_after_pre(git, cp, &pre)
        .await
        .map_err(|e| format!("{e} (pre_rollback_checkpoint_id={})", pre.id))
}

/// Everything after the pre-rollback checkpoint exists; any failure here is
/// reported together with that checkpoint's id so the caller can recover.
async fn rollback_after_pre(
    git: &Git,
    cp: &Checkpoint,
    pre: &Checkpoint,
) -> Result<RollbackResult, String> {
    // Everything that differs between "now" (the pre-rollback snapshot, which
    // holds untracked files too) and the target, as `status\0path\0` pairs.
    let changes = split_nul(
        &git.ok(&[
            "diff-tree",
            "-r",
            "--name-status",
            "-z",
            "--no-renames",
            &pre.snapshot_sha,
            &cp.snapshot_sha,
        ])
        .await?,
    );
    let mut restored = Vec::new();
    let mut gone = Vec::new();
    for pair in changes.chunks(2) {
        if let [status, path] = pair {
            if status.starts_with('D') {
                gone.push(path.clone());
            } else {
                restored.push(path.clone());
            }
        }
    }
    restored.sort();

    let source = format!("--source={}", cp.snapshot_sha);
    git.ok(&["restore", &source, "--worktree", "--staged", "--", "."])
        .await?;
    if !cp.index_tree.is_empty() {
        // Put the staged state back (index only; the worktree is already right).
        git.ok(&["read-tree", &cp.index_tree]).await?;
    }

    // Untracked now and absent from the snapshot: created after the checkpoint.
    let in_snapshot: HashSet<String> = split_nul(
        &git.ok(&["ls-tree", "-r", "--name-only", "-z", &cp.snapshot_sha])
            .await?,
    )
    .into_iter()
    .collect();
    for path in git.untracked().await? {
        if in_snapshot.contains(&path) {
            continue;
        }
        if let Err(e) = remove_untracked(git.root(), &path) {
            log::warn!("[debug_mode] could not remove untracked path: {e}");
        }
    }
    // Report what really is gone (includes staged-new files `restore` dropped).
    let mut removed: Vec<String> = gone
        .into_iter()
        .filter(|p| std::fs::symlink_metadata(git.root().join(p)).is_err())
        .collect();
    removed.sort();

    let head_moved = git.head().await?.as_deref() != Some(cp.head.as_str());
    log::debug!(
        "[debug_mode] rollback id={} restored={} removed={} head_moved={head_moved}",
        cp.id,
        restored.len(),
        removed.len()
    );
    Ok(RollbackResult {
        checkpoint_id: cp.id.clone(),
        pre_rollback_checkpoint_id: pre.id.clone(),
        restored,
        removed,
        head_moved,
    })
}

/// Delete one untracked file, refusing anything that is not a plain relative
/// path inside `root` (and never following a symlink out of it).
fn remove_untracked(root: &Path, rel: &str) -> Result<(), String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || !rel_path
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("unsafe path".into());
    }
    let full = root.join(rel_path);
    let parent = full.parent().ok_or("no parent")?;
    let parent = std::fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if !parent.starts_with(root) {
        return Err("path escapes project root".into());
    }
    let meta = std::fs::symlink_metadata(&full).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        return Err("is a directory".into());
    }
    std::fs::remove_file(&full).map_err(|e| e.to_string())?;
    // Drop directories this removal left empty (never root itself, never
    // anything outside it; `remove_dir` refuses non-empty directories).
    let mut dir = full.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
    Ok(())
}

fn parse_name_status(bytes: &[u8]) -> DiffSummary {
    let toks = split_nul(bytes);
    let mut s = DiffSummary::default();
    for pair in toks.chunks(2) {
        match pair[0].chars().next() {
            Some('A') => s.created += 1,
            Some('D') => s.deleted += 1,
            _ => s.modified += 1,
        }
    }
    s
}

fn parse_numstat(bytes: &[u8]) -> Vec<DiffFile> {
    split_nul(bytes)
        .into_iter()
        .filter_map(|t| {
            let mut it = t.splitn(3, '\t');
            let (a, d, path) = (it.next()?, it.next()?, it.next()?);
            Some(DiffFile {
                path: path.to_string(),
                added: a.parse().ok(),
                deleted: d.parse().ok(),
            })
        })
        .collect()
}

/// Working tree (tracked + untracked, ignored excluded) vs HEAD or a
/// checkpoint snapshot. New files appear in `text`/`files`/`summary`;
/// `untracked` additionally lists the files git does not track yet.
pub async fn diff(git: &Git, cp: Option<&Checkpoint>) -> Result<DiffResult, String> {
    let (base_label, base) = match cp {
        Some(c) => (c.id.clone(), format!("{}^{{tree}}", c.snapshot_sha)),
        None => {
            git.head().await?.ok_or("repository has no commits yet")?;
            ("HEAD".to_string(), "HEAD^{tree}".to_string())
        }
    };
    // Current working tree (incl. new files) via a throwaway index.
    let current = git.snapshot_trees().await?.worktree_tree;
    let common = ["diff", "--no-ext-diff", "--no-color", "--no-renames"];
    let with = |extra: &[&'static str]| -> Vec<String> {
        let mut v: Vec<String> = common.iter().map(|s| s.to_string()).collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v.push(base.clone());
        v.push(current.clone());
        v
    };
    let run = |args: Vec<String>, cap: usize| async move {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = git.run_cap(&refs, None, cap).await?;
        if !out.success() {
            return Err(format!("git diff failed (exit {:?})", out.exit_code));
        }
        Ok::<_, String>(out)
    };
    let names = run(with(&["--name-status", "-z"]), 8 << 20).await?;
    let nums = run(with(&["--numstat", "-z"]), 8 << 20).await?;
    let patch = run(with(&[]), DIFF_CAP).await?;

    let summary = parse_name_status(&names.stdout);
    let mut untracked = git.untracked().await?;
    if let Some(c) = cp {
        let at_cp: HashSet<&String> = c.untracked_files.iter().collect();
        untracked.retain(|p| !at_cp.contains(p));
    }
    Ok(DiffResult {
        base: base_label,
        text: String::from_utf8_lossy(&patch.stdout).into_owned(),
        truncated: patch.stdout_truncated,
        summary,
        files: parse_numstat(&nums.stdout),
        untracked,
    })
}
