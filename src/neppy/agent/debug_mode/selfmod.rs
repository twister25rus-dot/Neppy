//! Self-modification safety (spec section 35): which files are *critical* —
//! code the debug runtime itself depends on — and which are *protected*
//! (the recovery tool, spec section 37, which must be hard to modify).
//!
//! Patterns are repo-relative and glob-like but deliberately tiny: a trailing
//! `/**` matches everything under a directory, a trailing `*` matches any
//! suffix, anything else is an exact path. Matching is case-insensitive
//! (macOS and Windows file systems are, so `Cargo.TOML` is still `Cargo.toml`).

use super::types::SelfModAssessment;

/// Files whose change can break the debug runtime or the app's ability to
/// start. A task touching one needs a validated candidate before it may
/// report `pass`.
pub const CRITICAL_PATTERNS: &[&str] = &[
    "src/neppy/agent/debug_mode/**",
    "src/neppy/agent/registry/agents/debug_agent/**",
    "src/neppy/agent/turn_workspace.rs",
    "src/neppy/agent/turn_origin.rs",
    "src/neppy/security/**",
    "src/neppy/tools/impl/system/**",
    "src/neppy/web_chat/run_task.rs",
    "src/neppy/web_chat/mode.rs",
    "src/core/**",
    "src/main.rs",
    "src/lib.rs",
    "Cargo.toml",
    "Cargo.lock",
    "app/src-tauri/**",
    "app/src/features/debug/**",
    "app/src/pages/DebugPage.tsx",
    "updater/**",
    "scripts/release*",
    "scripts/neppy-recover.sh",
    "scripts/neppy-install-local.sh",
];

/// Files Debug Mode's own path policy must refuse to write (recovery has to
/// survive a bad self-modification).
pub const PROTECTED_PATTERNS: &[&str] =
    &["scripts/neppy-recover.sh", "scripts/neppy-install-local.sh"];

fn normalize(path: &str) -> String {
    let mut p = path.trim().replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    p.trim_start_matches('/').to_ascii_lowercase()
}

fn matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    if let Some(dir) = pattern.strip_suffix("/**") {
        return path == dir || path.starts_with(&format!("{dir}/"));
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return path.starts_with(prefix);
    }
    path == pattern
}

fn matches_any(patterns: &[&str], repo_rel: &str) -> bool {
    let path = normalize(repo_rel);
    // A path that climbs out of the tree is never inside a safe zone.
    if path.split('/').any(|c| c == "..") {
        return true;
    }
    patterns.iter().any(|p| matches(p, &path))
}

/// True when `repo_rel` (a repo-relative path) is a critical file.
pub fn is_critical_path(repo_rel: &str) -> bool {
    matches_any(CRITICAL_PATTERNS, repo_rel)
}

/// True when `repo_rel` is protected from modification by Debug Mode.
pub fn is_protected_path(repo_rel: &str) -> bool {
    matches_any(PROTECTED_PATTERNS, repo_rel)
}

/// Classifies a set of changed files. `critical_files` is sorted and unique.
pub fn assess(files: &[String]) -> SelfModAssessment {
    let mut critical_files: Vec<String> = files
        .iter()
        .filter(|f| is_critical_path(f))
        .cloned()
        .collect();
    critical_files.sort();
    critical_files.dedup();
    log::debug!(
        "[debug_mode] selfmod assess files={} critical={}",
        files.len(),
        critical_files.len()
    );
    SelfModAssessment {
        critical: !critical_files.is_empty(),
        critical_files,
    }
}

#[cfg(test)]
#[path = "selfmod_tests.rs"]
mod tests;
