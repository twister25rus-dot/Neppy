//! Applying the model's edits, safely and exactly once.
//!
//! A path is checked before anything else: relative, inside the project root,
//! not through a symlink, not under `.git`, not ignored or generated, and not
//! in the Neppy workspace. The autonomy tier decides whether edits are allowed
//! at all. The write itself is guarded by content hashes and lands through a
//! temp file and a rename, so a reader sees the old file or the new one.
//!
//! Each edit has a key derived from the task, step, position and content. The
//! ledger row is written *before* the file changes, with the hash the file had
//! and the hash it will have, so a resume after a crash can tell the three
//! cases apart: already written (skip), not yet written (write), or something
//! else changed it in the meantime (conflict, never overwritten).

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::neppy::config::schema::LocalAssistantConfig;
use crate::neppy::security::SecurityPolicy;

use super::faults::{check, FaultPoint, Faults};
use super::index::{is_generated, sha_hex};
use super::store::StateStore;
use super::types::*;

/// Hash recorded for a file that does not exist yet.
const ABSENT: &str = "absent";
const MAX_PATH_CHARS: usize = 512;

pub(crate) struct EditCtx<'a> {
    /// Canonical project root.
    pub(crate) root: &'a Path,
    pub(crate) policy: &'a SecurityPolicy,
    /// The Neppy workspace the state and index live in.
    pub(crate) workspace: &'a Path,
    pub(crate) cfg: &'a LocalAssistantConfig,
    pub(crate) store: &'a StateStore,
    pub(crate) task_id: &'a str,
    pub(crate) step_no: u32,
    pub(crate) faults: &'a dyn Faults,
}

/// Stable identity of one edit within one step of one task.
pub(crate) fn effect_key(task_id: &str, step_no: u32, idx: usize, edit: &EditOp) -> String {
    let mut h = Sha256::new();
    for part in [
        task_id.as_bytes(),
        &step_no.to_le_bytes(),
        &(idx as u64).to_le_bytes(),
        edit.path.as_bytes(),
        &[0],
        edit.search.as_bytes(),
        &[0],
        edit.replace.as_bytes(),
    ] {
        h.update(part);
    }
    hex::encode(h.finalize())
}

/// Identity of the test run of one step.
pub(crate) fn test_key(task_id: &str, step_no: u32) -> String {
    format!("{task_id}|{step_no}|test")
}

/// A relative, normal-components-only path, or why it was refused.
pub(crate) fn validate_rel(rel: &str) -> std::result::Result<PathBuf, String> {
    if rel.is_empty() || rel.len() > MAX_PATH_CHARS {
        return Err("path is empty or too long".into());
    }
    if rel.contains('\0') || rel.contains('\\') || rel.starts_with('~') {
        return Err("path contains a forbidden character".into());
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".into());
    }
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                if part.to_string_lossy().eq_ignore_ascii_case(".git") {
                    return Err("paths under .git are not allowed".into());
                }
            }
            Component::ParentDir => return Err("`..` is not allowed".into()),
            _ => return Err("path must be plain relative components".into()),
        }
    }
    Ok(path.to_path_buf())
}

/// Whether git (if this is a repository) says `rel` is ignored.
fn git_ignored(root: &Path, rel: &str) -> bool {
    if !root.join(".git").exists() {
        return false;
    }
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "-q", "--", rel])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.code() == Some(0))
        .unwrap_or(false)
}

/// The absolute target, or why the edit is refused. Nothing is created.
fn resolve_target(ctx: &EditCtx<'_>, rel: &str) -> std::result::Result<PathBuf, String> {
    let rel_path = validate_rel(rel)?;
    if !ctx.policy.can_act() {
        return Err("the autonomy tier is read-only; edits are not allowed".into());
    }
    let mut walk = ctx.root.to_path_buf();
    for component in rel_path.components() {
        walk.push(component);
        match std::fs::symlink_metadata(&walk) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("the path goes through a symlink".into())
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let abs = ctx.root.join(&rel_path);
    if let Some(parent) = abs.parent() {
        if let Ok(canon) = parent.canonicalize() {
            if !canon.starts_with(ctx.root) {
                return Err("the path escapes the project root".into());
            }
        }
    }
    let rel_str = rel_path.to_string_lossy().replace('\\', "/");
    if super::index::is_generated_rel(&rel_str) {
        return Err("generated or build-output paths are not editable".into());
    }
    if super::index::exclude_matches(&ctx.cfg.exclude_globs, &rel_str) {
        return Err("the path is excluded by configuration".into());
    }
    if git_ignored(ctx.root, &rel_str) {
        return Err("the path is ignored by git".into());
    }
    if SecurityPolicy::is_always_forbidden(&abs) {
        return Err("the path is a protected location".into());
    }
    let workspaces = [ctx.workspace, ctx.policy.workspace_dir.as_path()];
    for ws in workspaces {
        let canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
        if abs.starts_with(&canon) || abs.starts_with(ws) {
            return Err("the Neppy workspace is not editable".into());
        }
    }
    if ctx.policy.is_workspace_internal_path(&abs) {
        return Err("the path is internal application state".into());
    }
    Ok(abs)
}

fn unique_replacement(current: &str, edit: &EditOp) -> std::result::Result<String, String> {
    if edit.search.is_empty() {
        return Err("an empty `search` is only valid for creating a new file".into());
    }
    match current.matches(edit.search.as_str()).count() {
        0 => Err("the search text was not found".into()),
        1 => Ok(current.replacen(edit.search.as_str(), &edit.replace, 1)),
        n => Err(format!(
            "the search text matches {n} places; it must be unique"
        )),
    }
}

/// Write `bytes` to `target` through a temp file in the same directory.
fn write_atomic(target: &Path, bytes: &[u8], mode_from: Option<&std::fs::Metadata>) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| AssistantError::Io("edit target has no parent".into()))?;
    std::fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{name}.neppy-{}.tmp",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(meta) = mode_from {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, target)
    })();
    if let Err(err) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn record(ctx: &EditCtx<'_>, key: &str, edit: &EditOp, pre: &str, post: &str) -> Result<()> {
    ctx.store.record_effect_intent(&EffectRecord {
        effect_key: key.to_string(),
        task_id: ctx.task_id.to_string(),
        step_no: ctx.step_no,
        kind: EffectKind::Edit,
        path: edit.path.clone(),
        pre_sha: pre.to_string(),
        post_sha: post.to_string(),
        status: EffectStatus::Intent,
        result: None,
    })
}

fn conflict(ctx: &EditCtx<'_>, key: &str, edit: &EditOp, why: String) -> Result<EditOutcome> {
    record(ctx, key, edit, "", "")?;
    ctx.store
        .mark_effect(key, EffectStatus::Conflict, Some(&why))?;
    log::warn!(
        "[local_assistant:edit] task {} step {} edit on `{}` conflicts: {why}",
        ctx.task_id,
        ctx.step_no,
        edit.path
    );
    Ok(EditOutcome::Conflict(why))
}

fn refuse(ctx: &EditCtx<'_>, key: &str, edit: &EditOp, why: String) -> Result<EditOutcome> {
    record(ctx, key, edit, "", "")?;
    ctx.store
        .mark_effect(key, EffectStatus::Refused, Some(&why))?;
    log::warn!(
        "[local_assistant:edit] task {} step {} edit on `{}` refused: {why}",
        ctx.task_id,
        ctx.step_no,
        edit.path
    );
    Ok(EditOutcome::Refused(why))
}

/// Record an edit as refused without looking at the disk, for a task that has
/// edits switched off.
pub(crate) fn refuse_disabled(
    ctx: &EditCtx<'_>,
    idx: usize,
    edit: &EditOp,
    why: &str,
) -> Result<EditOutcome> {
    let key = effect_key(ctx.task_id, ctx.step_no, idx, edit);
    refuse(ctx, &key, edit, why.to_string())
}

/// Apply edit number `idx` of the step, at most once however often this is
/// called for the same step.
pub(crate) fn apply_edit(ctx: &EditCtx<'_>, idx: usize, edit: &EditOp) -> Result<EditOutcome> {
    let key = effect_key(ctx.task_id, ctx.step_no, idx, edit);
    let target = match resolve_target(ctx, &edit.path) {
        Ok(target) => target,
        Err(why) => return refuse(ctx, &key, edit, why),
    };
    let prior = ctx.store.get_effect(&key)?;
    if let Some(prior) = &prior {
        match prior.status {
            EffectStatus::Applied | EffectStatus::Done => {
                log::debug!(
                    "[local_assistant:edit] {} already applied; skipping",
                    &key[..12]
                );
                return Ok(EditOutcome::AlreadyApplied);
            }
            EffectStatus::Conflict => {
                return Ok(EditOutcome::Conflict(
                    prior.result.clone().unwrap_or_default(),
                ));
            }
            EffectStatus::Intent | EffectStatus::Refused => {}
        }
    }

    let meta = std::fs::symlink_metadata(&target).ok();
    let current = match &meta {
        Some(m) if m.is_file() => {
            if m.len() > ctx.cfg.index_max_file_bytes {
                return refuse(ctx, &key, edit, "the file is too large to edit".into());
            }
            Some(std::fs::read(&target)?)
        }
        Some(_) => return refuse(ctx, &key, edit, "the path is not a regular file".into()),
        None => None,
    };
    if let Some(bytes) = &current {
        if is_generated(&edit.path, bytes) {
            return refuse(ctx, &key, edit, "the file is generated or binary".into());
        }
    }
    let pre_sha = current
        .as_deref()
        .map_or_else(|| ABSENT.to_string(), sha_hex);

    // Work out the file the edit produces from the file as it is now.
    let new_text = match (&current, edit.search.is_empty()) {
        (None, true) => Ok(edit.replace.clone()),
        (None, false) => Err("the file does not exist".to_string()),
        (Some(_), true) => Err("the file already exists".to_string()),
        (Some(bytes), false) => match std::str::from_utf8(bytes) {
            Ok(text) => unique_replacement(text, edit),
            Err(_) => Err("the file is not UTF-8 text".to_string()),
        },
    };

    if let Some(prior) = prior.filter(|p| p.status == EffectStatus::Intent) {
        // A crash left an intent behind. Decide from what is on disk.
        if pre_sha == prior.post_sha {
            ctx.store.mark_effect(&key, EffectStatus::Applied, None)?;
            log::info!(
                "[local_assistant:edit] {} was written before the crash",
                &key[..12]
            );
            return Ok(EditOutcome::AlreadyApplied);
        }
        if pre_sha != prior.pre_sha {
            return conflict(
                ctx,
                &key,
                edit,
                "the file changed since the edit was planned".into(),
            );
        }
    }
    let new_text = match new_text {
        Ok(text) => text,
        Err(why) => return conflict(ctx, &key, edit, why),
    };
    if new_text.len() as u64 > ctx.cfg.index_max_file_bytes {
        return refuse(ctx, &key, edit, "the result would be too large".into());
    }
    let post_sha = sha_hex(new_text.as_bytes());
    record(ctx, &key, edit, &pre_sha, &post_sha)?;
    check(ctx.faults, FaultPoint::Intent(idx))?;

    if pre_sha == post_sha {
        ctx.store.mark_effect(&key, EffectStatus::Applied, None)?;
        return Ok(EditOutcome::Applied);
    }
    // Narrow the window: confirm the file is still the one the hashes describe.
    let still = std::fs::read(&target)
        .ok()
        .as_deref()
        .map_or_else(|| ABSENT.to_string(), sha_hex);
    if still != pre_sha {
        return conflict(
            ctx,
            &key,
            edit,
            "the file changed while the edit was prepared".into(),
        );
    }
    write_atomic(&target, new_text.as_bytes(), meta.as_ref())?;
    check(ctx.faults, FaultPoint::Write(idx))?;
    ctx.store.mark_effect(&key, EffectStatus::Applied, None)?;
    log::info!(
        "[local_assistant:edit] task {} step {} edit {} applied to `{}` ({} -> {} bytes)",
        ctx.task_id,
        ctx.step_no,
        idx,
        edit.path,
        current.as_ref().map_or(0, Vec::len),
        new_text.len()
    );
    Ok(EditOutcome::Applied)
}

#[cfg(test)]
#[path = "edits_tests.rs"]
mod tests;
