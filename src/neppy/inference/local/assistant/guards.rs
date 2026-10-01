//! Path policy for the assistant: which project roots it may work on, and
//! which files inside one it may edit.
//!
//! The agent's file tools gate a *write* on `is_resolved_path_allowed_for(path,
//! true)` (the workspace or a read-write trusted root), on the configured
//! `forbidden_paths`, and on the always-forbidden credential and OS locations.
//! The assistant used to check only the last of those, so any directory the
//! caller named could be written to. These functions apply the same rules, and
//! are re-run at run time, not just when a task is accepted: a task can sit in
//! the queue for a long time, and the config or the tree can change under it.
//!
//! Three things are deliberately stricter than the tools:
//! - a root may not be `/`, the home directory, or anything that contains it;
//! - a root that is edited must be inside a git repository, so every change is
//!   reviewable and revertable;
//! - paths that run code without the user asking (git hooks, editor tasks, CI
//!   workflows, build scripts, `.envrc`, `package.json`) are not editable unless
//!   `local_assistant.allow_sensitive_paths` is set. A model reading a hostile
//!   repository must not be able to plant something the user's next `cargo
//!   build` or `git commit` runs.

use std::path::{Path, PathBuf};

use crate::neppy::config::schema::LocalAssistantConfig;
use crate::neppy::security::policy::TrustedAccess;
use crate::neppy::security::SecurityPolicy;

/// Directory-name sequences (case-insensitive) that are autostart or
/// persistence locations. Refused as a root and as an edit target even inside
/// a trusted root.
const PERSISTENCE_SEQUENCES: &[&[&str]] = &[
    &["library", "launchagents"],
    &["library", "launchdaemons"],
    &[".config", "autostart"],
    &["start menu", "programs", "startup"],
];

/// Directory components whose contents are executed by tooling on its own.
const SENSITIVE_DIRS: &[&str] = &[
    ".husky",
    ".githooks",
    ".vscode",
    ".idea",
    ".devcontainer",
    ".circleci",
];
/// File names (any directory) that run code when a tool opens the project.
const SENSITIVE_FILES: &[&str] = &[
    ".envrc",
    "build.rs",
    "package.json",
    ".npmrc",
    ".yarnrc",
    ".yarnrc.yml",
    ".pre-commit-config.yaml",
    ".gitlab-ci.yml",
];

/// Why `rel` (project-relative, `/`-separated) is a sensitive path, or `None`.
pub(crate) fn sensitive_reason(rel: &str) -> Option<&'static str> {
    let lower = rel.to_ascii_lowercase();
    let parts: Vec<&str> = lower.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let is_last = i + 1 == parts.len();
        if !is_last && SENSITIVE_DIRS.contains(part) {
            return Some("git hooks, editor and CI configuration run code on their own");
        }
        if !is_last && *part == ".github" && parts.get(i + 1) == Some(&"workflows") {
            return Some("CI workflows run code on their own");
        }
        if !is_last
            && *part == ".cargo"
            && parts.get(i + 1).is_some_and(|f| f.starts_with("config"))
        {
            return Some("cargo configuration can run arbitrary programs");
        }
        if !is_last && *part == ".git" {
            return Some("git internals");
        }
        if is_last && SENSITIVE_FILES.contains(part) {
            return Some("this file is run by build or package tooling");
        }
    }
    None
}

fn contains_sequence(segments: &[String], needle: &[&str]) -> bool {
    segments
        .windows(needle.len())
        .any(|w| w.iter().zip(needle).all(|(a, b)| a == b))
}

/// Whether `path` is under an autostart or persistence location.
pub(crate) fn is_persistence_path(path: &Path) -> bool {
    let segments: Vec<String> = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    PERSISTENCE_SEQUENCES
        .iter()
        .any(|seq| contains_sequence(&segments, seq))
}

fn expand(raw: &str) -> PathBuf {
    PathBuf::from(crate::neppy::config::expand_tilde(raw))
}

/// A path in the forms the policy compares it in: as written and canonical.
fn forms(path: PathBuf) -> Vec<PathBuf> {
    let canon = path.canonicalize().ok();
    let mut out = vec![path];
    if let Some(canon) = canon {
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    out
}

fn trusted_paths(policy: &SecurityPolicy, write: bool) -> Vec<PathBuf> {
    policy
        .trusted_roots
        .iter()
        .filter(|r| !write || r.access == TrustedAccess::ReadWrite)
        .flat_map(|r| forms(expand(&r.path)))
        .collect()
}

/// The configured `forbidden_paths` entry that covers `target`, if any.
///
/// A trusted root that sits at or below a forbidden entry carves it out (the
/// policy's own precedence: a user who granted `/tmp/openhuman` meant it). A
/// forbidden entry that sits *inside* a trusted root is the more specific rule
/// and wins, so a project can fence off a subdirectory of its own tree.
pub(crate) fn forbidden_hit(policy: &SecurityPolicy, target: &Path, write: bool) -> Option<String> {
    let trusted = trusted_paths(policy, write);
    for entry in &policy.forbidden_paths {
        for forbidden in forms(expand(entry)) {
            if !target.starts_with(&forbidden) {
                continue;
            }
            let carved_out = trusted
                .iter()
                .any(|root| root.starts_with(&forbidden) && target.starts_with(root));
            if !carved_out {
                return Some(entry.clone());
            }
        }
    }
    None
}

/// The nearest existing ancestor of `path`, canonicalized, with the missing
/// tail re-appended, so a not-yet-created file is judged where it would land.
fn canonical_landing(path: &Path) -> PathBuf {
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        if let Ok(canon) = cursor.canonicalize() {
            let mut out = canon;
            for part in missing.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cursor.file_name().map(ToOwned::to_owned), cursor.parent()) {
            (Some(name), Some(parent)) => {
                missing.push(name);
                cursor = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn overlaps_workspace(policy: &SecurityPolicy, workspace: &Path, canon: &Path) -> bool {
    [workspace, policy.workspace_dir.as_path()]
        .iter()
        .any(|ws| {
            let ws_canon = ws.canonicalize().unwrap_or_else(|_| ws.to_path_buf());
            canon.starts_with(&ws_canon) || ws_canon.starts_with(canon)
        })
}

/// The directory at or above `root` that holds `.git`, if any.
fn repository_top(root: &Path) -> Option<PathBuf> {
    root.ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf)
}

fn is_home_or_above(path: &Path, home: Option<&Path>) -> bool {
    path.parent().is_none() || home.is_some_and(|h| h.starts_with(path))
}

/// A directory the assistant may work on, or why not. `allow_edits` selects
/// the stricter write rules.
pub(crate) fn validate_root(
    policy: &SecurityPolicy,
    workspace: &Path,
    root: &Path,
    allow_edits: bool,
) -> std::result::Result<PathBuf, String> {
    let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    validate_root_with_home(policy, workspace, root, allow_edits, home.as_deref())
}

pub(crate) fn validate_root_with_home(
    policy: &SecurityPolicy,
    workspace: &Path,
    root: &Path,
    allow_edits: bool,
    home: Option<&Path>,
) -> std::result::Result<PathBuf, String> {
    let canon = root
        .canonicalize()
        .map_err(|e| format!("project_root `{}`: {e}", root.display()))?;
    if !canon.is_dir() {
        return Err("project_root is not a directory".into());
    }
    if is_home_or_above(&canon, home) {
        return Err(
            "project_root must be a project directory, not the filesystem root, the home \
             directory or a directory that contains it"
                .into(),
        );
    }
    if SecurityPolicy::is_always_forbidden(&canon) {
        return Err("project_root is a protected location".into());
    }
    if overlaps_workspace(policy, workspace, &canon) {
        return Err("project_root must not contain, or be inside, the Neppy workspace".into());
    }
    if !allow_edits {
        return Ok(canon);
    }
    if is_persistence_path(&canon) {
        return Err("project_root is an autostart or persistence location".into());
    }
    if !policy.can_act() {
        return Err("the autonomy tier is read-only; edits are not allowed".into());
    }
    if let Some(entry) = forbidden_hit(policy, &canon, true) {
        return Err(format!(
            "project_root is under the forbidden path `{entry}` (autonomy.forbidden_paths)"
        ));
    }
    if !policy.is_resolved_path_allowed_for(&canon, true) {
        return Err(
            "project_root is not a read-write location for the agent: add it to \
             autonomy.trusted_roots with access `readwrite` to let the assistant edit it"
                .into(),
        );
    }
    match repository_top(&canon) {
        None => Err(
            "project_root is not inside a git repository; the assistant only edits \
                     projects whose changes git can review and revert"
                .into(),
        ),
        Some(top) if is_home_or_above(&top, home) => Err(
            "the git repository containing project_root is the home directory or above; \
             pick a project inside it"
                .into(),
        ),
        Some(_) => Ok(canon),
    }
}

/// Re-check, for one edit target, everything the root check established plus
/// the path-specific rules. `abs` is `root` joined with `rel`.
pub(crate) fn check_edit_target(
    policy: &SecurityPolicy,
    cfg: &LocalAssistantConfig,
    rel: &str,
    abs: &Path,
) -> std::result::Result<(), String> {
    if !cfg.allow_sensitive_paths {
        if let Some(why) = sensitive_reason(rel) {
            return Err(format!(
                "`{rel}` is not editable: {why} (set local_assistant.allow_sensitive_paths to allow)"
            ));
        }
    }
    let landing = canonical_landing(abs);
    if is_persistence_path(&landing) {
        return Err("the path is an autostart or persistence location".into());
    }
    if let Some(entry) = forbidden_hit(policy, &landing, true) {
        return Err(format!("the path is under the forbidden path `{entry}`"));
    }
    if !policy.is_resolved_path_allowed_for(&landing, true) {
        return Err("the path is not a read-write location for the agent".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "guards_tests.rs"]
mod tests;
