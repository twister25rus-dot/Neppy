//! Which files belong in the index.
//!
//! The candidate list comes from git, so `.gitignore`, `.git/info/exclude` and
//! the global excludes are honoured by the tool that owns them rather than by
//! a second implementation here. A directory that is not a repository falls
//! back to a walk with a fixed skip list.

use std::path::Path;
use std::process::{Command, Stdio};

use glob::{MatchOptions, Pattern};

use super::super::types::{AssistantError, Result};

/// Directory names that hold build output or dependencies.
const GENERATED_DIRS: &[&str] = &[
    "target",
    "dist",
    "build",
    "node_modules",
    ".next",
    "coverage",
];
/// Directories never descended into by the non-git fallback.
const FALLBACK_SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "dist",
    "build",
    "node_modules",
    ".next",
    "coverage",
    ".venv",
    "__pycache__",
    ".cache",
];
const GENERATED_SUFFIXES: &[&str] = &[".lock", ".map", ".snap"];
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "tiff", "pdf", "zip", "gz", "tar", "tgz",
    "bz2", "xz", "7z", "rar", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov", "avi",
    "wav", "ogg", "flac", "wasm", "so", "dylib", "dll", "exe", "bin", "class", "jar", "o", "a",
    "rlib", "dmg", "pkg", "icns", "psd", "sqlite", "db",
];
const GENERATED_MARKERS: &[&str] = &["@generated", "DO NOT EDIT", "<!-- BEGIN GENERATED"];
/// Lines at the top of a file searched for a generated-file marker.
const MARKER_LINES: usize = 5;
/// Bytes at the top of a file inspected for NUL.
pub const BINARY_PROBE_BYTES: usize = 8192;
/// Stop a fallback walk after this many entries.
const FALLBACK_MAX_ENTRIES: usize = 500_000;

/// The git commit the project is at, if it is a repository.
pub(crate) fn git_head(root: &Path) -> Option<String> {
    let out = git(root).args(["rev-parse", "HEAD"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!head.is_empty()).then_some(head)
}

fn git(root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        // A hook that runs tests exports these; they would point git at some
        // other repository.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    cmd
}

/// Project-relative paths (`/` separated) that could be indexed, sorted.
/// Still unfiltered by size, content or generated-ness.
pub(crate) fn list_candidates(root: &Path) -> Result<Vec<String>> {
    if let Some(listed) = list_with_git(root) {
        log::debug!(
            "[local_assistant:index] git listed {} candidate paths",
            listed.len()
        );
        return Ok(listed);
    }
    let walked = list_with_walk(root)?;
    log::debug!(
        "[local_assistant:index] walk listed {} candidate paths (not a git repo)",
        walked.len()
    );
    Ok(walked)
}

fn list_with_git(root: &Path) -> Option<Vec<String>> {
    let out = git(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut paths: Vec<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|raw| !raw.is_empty())
        .filter_map(|raw| std::str::from_utf8(raw).ok())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    Some(paths)
}

fn list_with_walk(root: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 || !entry.file_type().is_dir() {
                return true;
            }
            !entry
                .file_name()
                .to_str()
                .is_some_and(|name| FALLBACK_SKIP_DIRS.contains(&name))
        });
    for (seen, entry) in walker.enumerate() {
        if seen >= FALLBACK_MAX_ENTRIES {
            log::warn!("[local_assistant:index] walk stopped at {FALLBACK_MAX_ENTRIES} entries");
            break;
        }
        let entry = entry.map_err(|err| AssistantError::Io(err.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            if let Some(rel) = rel.to_str() {
                out.push(rel.replace(std::path::MAIN_SEPARATOR, "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Whether the path alone marks the file as build output or a lockfile.
pub(crate) fn is_generated_path(rel: &str) -> bool {
    let mut parts = rel.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_some() {
            // Case-insensitive: `Target/`, `Build/` and `NODE_MODULES/` are the
            // same directory on a case-insensitive filesystem.
            if GENERATED_DIRS
                .iter()
                .any(|dir| dir.eq_ignore_ascii_case(part))
            {
                return true;
            }
        } else {
            let lower = part.to_ascii_lowercase();
            if GENERATED_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
                return true;
            }
            // `app.min.js`, `site.min.css`
            if lower.contains(".min.") {
                return true;
            }
        }
    }
    false
}

/// Whether the file is generated, by path or by a marker near its top.
pub(crate) fn is_generated(rel: &str, head: &[u8]) -> bool {
    if is_generated_path(rel) {
        return true;
    }
    let text = String::from_utf8_lossy(&head[..head.len().min(BINARY_PROBE_BYTES)]);
    text.lines()
        .take(MARKER_LINES)
        .any(|line| GENERATED_MARKERS.iter().any(|m| line.contains(m)))
}

/// Whether the file should be treated as binary: a known extension, or NUL in
/// the probe window.
pub(crate) fn looks_binary(rel: &str, head: &[u8]) -> bool {
    let ext = rel
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if BINARY_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    head[..head.len().min(BINARY_PROBE_BYTES)].contains(&0)
}

/// User-configured exclusions, matched against project-relative paths.
pub(crate) struct ExcludeSet {
    patterns: Vec<Pattern>,
}

impl ExcludeSet {
    pub(crate) fn new(globs: &[String]) -> Self {
        let patterns = globs
            .iter()
            .filter_map(|g| match Pattern::new(g) {
                Ok(p) => Some(p),
                Err(err) => {
                    log::warn!("[local_assistant:index] ignoring bad exclude glob `{g}`: {err}");
                    None
                }
            })
            .collect();
        Self { patterns }
    }

    pub(crate) fn matches(&self, rel: &str) -> bool {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        self.patterns
            .iter()
            .any(|p| p.matches_with(rel, options) || p.matches_with(&format!("{rel}/"), options))
    }
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
