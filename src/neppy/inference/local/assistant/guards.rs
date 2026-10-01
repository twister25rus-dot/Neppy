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
//!   configuration, build and package manifests and scripts, dotfiles a shell
//!   reads, bare-repository files; the full list is `SENSITIVE_*` below) are
//!   not editable unless `local_assistant.allow_sensitive_paths` is set. A
//!   model reading a hostile repository must not be able to plant something the
//!   user's next `cargo build` or `git commit` runs.
//!
//! **Names are not trusted as written.** A macOS or Windows filesystem looks
//! names up case-insensitively and with Unicode folding (APFS resolves `.huſky`
//! to `.husky` and `package.jſon` to `package.json`), so a denylist that
//! compares the model's spelling can be walked around. Two measures, both
//! applied to every edit:
//! - the check runs on the name each component has *on disk* (found by inode,
//!   so no folding rule has to be known), as well as on the spelling given;
//! - an edit path with any non-ASCII component is refused by default, which
//!   closes the class rather than listing lookalikes. `allow_sensitive_paths`
//!   lifts it, because at that point the sensitive names are editable anyway.

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

/// Directory components whose contents are executed by tooling on its own, or
/// that configure it. Any file below one is sensitive.
const SENSITIVE_DIRS: &[&str] = &[
    ".husky",
    ".githooks",
    ".vscode",
    ".idea",
    ".devcontainer",
    ".circleci",
    // Actions, workflows, and the rest of the GitHub configuration.
    ".github",
    // Yarn releases and plugins are JavaScript Yarn runs.
    ".yarn",
];

/// File names (any directory) that run code when a tool opens, builds or tests
/// the project, or that a shell reads. Compared in folded lower case.
const SENSITIVE_FILES: &[&str] = &[
    // Version managers and environments.
    ".envrc",
    ".mise.toml",
    ".tool-versions",
    // JavaScript.
    "package.json",
    ".npmrc",
    ".yarnrc",
    ".yarnrc.yml",
    ".pnpmfile.cjs",
    // Rust: `build =` in Cargo.toml can name any file as a build script.
    "build.rs",
    "cargo.toml",
    // Make and friends.
    "makefile",
    "gnumakefile",
    "justfile",
    "rakefile",
    "gemfile",
    // Python.
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "conftest.py",
    "tox.ini",
    "noxfile.py",
    // Other build systems.
    "cmakelists.txt",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "pom.xml",
    // CI.
    ".pre-commit-config.yaml",
    ".gitlab-ci.yml",
    // Dotfiles a shell or editor reads on its own.
    ".bashrc",
    ".bash_profile",
    ".bash_login",
    ".bash_logout",
    ".profile",
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
    ".inputrc",
    ".gitconfig",
    ".gitmodules",
    // A bare repository planted inside the project: `HEAD` and `config` are
    // what make a directory a git dir, and `config` can name a program to run
    // (`core.fsmonitor`). Refused wherever they appear.
    "head",
    "config",
];

/// File-name suffixes that are sensitive: included makefiles.
const SENSITIVE_SUFFIXES: &[&str] = &[".mk"];

/// Full Unicode case folding as far as file names need it: lower case, plus
/// the folds `to_lowercase` leaves out (`ſ` is `s`; the Kelvin sign already
/// lowers to `k`). Used for the denylist comparison only.
fn fold(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|c| if c == '\u{17F}' { 's' } else { c })
        .collect()
}

/// A path component with a character outside ASCII.
fn first_non_ascii_component(rel: &str) -> Option<String> {
    rel.split('/')
        .find(|part| !part.is_ascii())
        .map(str::to_string)
}

/// Why `rel` (project-relative, `/`-separated) is a sensitive path, or `None`.
/// Compares Unicode-folded, so it is correct for the spelling it is given;
/// [`check_edit_target`] also runs it on the on-disk spelling.
pub(crate) fn sensitive_reason(rel: &str) -> Option<&'static str> {
    let folded = fold(rel);
    let parts: Vec<&str> = folded.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        let is_last = i + 1 == parts.len();
        if !is_last && SENSITIVE_DIRS.contains(part) {
            return Some(
                "git hooks, editor, CI and package-manager configuration run code on their own",
            );
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
        if part.len() > 4 && part.ends_with(".git") {
            return Some("a directory that looks like a bare git repository");
        }
        if is_last
            && (SENSITIVE_FILES.contains(part)
                || SENSITIVE_SUFFIXES.iter().any(|s| part.ends_with(s)))
        {
            return Some("this file is read or run by build, package, shell or git tooling");
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
    let segments: Vec<String> = fold(&path.to_string_lossy())
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

/// What a project root is going to be used for, which decides how strict the
/// check is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RootUse {
    /// Indexed and read. Nothing under it changes.
    Read,
    /// The user's test command runs in it. A command can write anywhere under
    /// the directory (`cargo test` fills `target/`), so it gets the write-root
    /// checks, minus the repository requirement: nothing is edited.
    Run,
    /// The model's edits are applied in it.
    Edit,
}

impl RootUse {
    /// The use a task needs: edits win, then a test command, else reading. A
    /// tier that cannot act runs nothing and edits nothing.
    pub(crate) fn for_task(edits: bool, has_command: bool, can_act: bool) -> Self {
        match (can_act, edits, has_command) {
            (false, ..) => Self::Read,
            (true, true, _) => Self::Edit,
            (true, false, true) => Self::Run,
            (true, false, false) => Self::Read,
        }
    }
}

/// A directory the assistant may work on, or why not.
pub(crate) fn validate_root(
    policy: &SecurityPolicy,
    workspace: &Path,
    root: &Path,
    usage: RootUse,
) -> std::result::Result<PathBuf, String> {
    let home = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    validate_root_with_home(policy, workspace, root, usage, home.as_deref())
}

pub(crate) fn validate_root_with_home(
    policy: &SecurityPolicy,
    workspace: &Path,
    root: &Path,
    usage: RootUse,
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
    if usage == RootUse::Read {
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
             autonomy.trusted_roots with access `readwrite` to let the assistant edit it or \
             run a test command in it"
                .into(),
        );
    }
    if usage == RootUse::Run {
        return Ok(canon);
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

/// The names `rel`'s components have on disk, where they exist.
///
/// Each existing component is matched to its directory entry by inode, so the
/// answer does not depend on knowing the filesystem's case or Unicode folding
/// rules: whatever the lookup resolved `.huſky` to, that entry's real name is
/// returned. Components that do not exist yet keep the spelling given.
#[cfg(unix)]
fn on_disk_rel(root: &Path, rel: &Path) -> String {
    use std::os::unix::fs::{DirEntryExt, MetadataExt};
    let mut dir = root.to_path_buf();
    let mut names: Vec<String> = Vec::new();
    let mut exists = true;
    for component in rel.components() {
        let given = component.as_os_str().to_string_lossy().into_owned();
        let mut real = given.clone();
        if exists {
            match std::fs::symlink_metadata(dir.join(&given)) {
                Ok(meta) => {
                    if let Ok(entries) = std::fs::read_dir(&dir) {
                        let found = entries
                            .flatten()
                            .find(|e| e.ino() == meta.ino())
                            .map(|e| e.file_name().to_string_lossy().into_owned());
                        if let Some(name) = found {
                            real = name;
                        }
                    }
                }
                Err(_) => exists = false,
            }
        }
        dir.push(&real);
        names.push(real);
    }
    names.join("/")
}

#[cfg(not(unix))]
fn on_disk_rel(_root: &Path, rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// Re-check, for one edit target, everything the root check established plus
/// the path-specific rules. `root` is the canonical project root and `abs` is
/// `root` joined with `rel`.
pub(crate) fn check_edit_target(
    policy: &SecurityPolicy,
    cfg: &LocalAssistantConfig,
    root: &Path,
    rel: &str,
    abs: &Path,
) -> std::result::Result<(), String> {
    if !cfg.allow_sensitive_paths {
        // The spelling the model gave, and the one the filesystem will act on.
        let real = on_disk_rel(root, Path::new(rel));
        for spelling in [rel, real.as_str()] {
            if let Some(part) = first_non_ascii_component(spelling) {
                return Err(format!(
                    "`{part}` contains non-ASCII characters; the assistant does not edit such \
                     paths, because filesystems fold them onto other names (set \
                     local_assistant.allow_sensitive_paths to allow)"
                ));
            }
            if let Some(why) = sensitive_reason(spelling) {
                return Err(format!(
                    "`{rel}` is not editable: {why} (set local_assistant.allow_sensitive_paths to allow)"
                ));
            }
        }
    }
    let landing = canonical_landing(abs);
    // The canonical landing, relative to the root, is a third spelling of the
    // same target: symlinked directories and every folding the lookup did.
    if !cfg.allow_sensitive_paths {
        if let Ok(tail) = landing.strip_prefix(root) {
            let tail = tail.to_string_lossy().replace('\\', "/");
            if let Some(why) = sensitive_reason(&tail) {
                return Err(format!(
                    "`{rel}` is not editable: it resolves to `{tail}`: {why} (set \
                     local_assistant.allow_sensitive_paths to allow)"
                ));
            }
            if first_non_ascii_component(&tail).is_some() {
                return Err(format!(
                    "`{rel}` resolves to a path with non-ASCII characters; refused"
                ));
            }
        }
    }
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
