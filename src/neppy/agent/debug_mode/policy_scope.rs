//! Scope-side half of Debug Mode policy: external directories and the prompt
//! addendum. Re-exported from [`super::policy`].

use std::path::{Path, PathBuf};

use crate::neppy::config::schema::debug_mode::DebugModeConfig;

// ── external directories ────────────────────────────────────────────────

/// Validates one external path: absolute, an existing directory, not `/`, not
/// the home directory itself, not an always-forbidden location. Returns the
/// canonical path.
pub fn validate_external_path(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let p = Path::new(raw);
    if raw.is_empty() || !p.is_absolute() {
        return Err(format!("external path '{raw}' must be an absolute path"));
    }
    let canon = std::fs::canonicalize(p)
        .map_err(|e| format!("external path '{raw}' is not accessible: {e}"))?;
    if !canon.is_dir() {
        return Err(format!("external path '{raw}' is not a directory"));
    }
    let is_home = dirs::home_dir()
        .and_then(|h| std::fs::canonicalize(h).ok())
        .is_some_and(|h| h == canon);
    if canon.parent().is_none() || is_home {
        return Err(format!(
            "external path '{raw}' is too broad (the filesystem root and the home directory are not allowed)"
        ));
    }
    if crate::neppy::security::SecurityPolicy::is_always_forbidden(&canon)
        || crate::neppy::security::SecurityPolicy::is_always_forbidden(p)
    {
        return Err(format!(
            "external path '{raw}' is a protected system or credential location"
        ));
    }
    Ok(canon)
}

/// The extra roots a Debug turn gets: empty unless `allow_external_filesystem`.
/// Paths that no longer validate are skipped (logged), never trusted.
pub fn external_roots(cfg: &DebugModeConfig) -> Vec<PathBuf> {
    if !cfg.allow_external_filesystem {
        return Vec::new();
    }
    let mut roots: Vec<PathBuf> = Vec::new();
    for raw in &cfg.external_paths {
        match validate_external_path(raw) {
            Ok(p) if !roots.contains(&p) => roots.push(p),
            Ok(_) => {}
            Err(e) => log::warn!("[debug_mode] external path skipped: {e}"),
        }
    }
    log::debug!("[debug_mode] external roots granted: {}", roots.len());
    roots
}

// ── prompt ──────────────────────────────────────────────────────────────

/// Config-derived lines appended to the Debug thread prompt.
pub fn prompt_addendum(cfg: &DebugModeConfig) -> String {
    let yn = |b: bool| if b { "yes" } else { "no" };
    let mut s = format!(
        "Debug mode settings for this session:\n- Repair attempts: {} at most{}.\n- After changes run tests: {}; run the build: {}.\n- Installing dependencies: {}. Git push: {}. Git commit: {}.",
        cfg.repair_iterations(),
        if cfg.auto_repair { "" } else { " (auto-repair is off: report failures instead of retrying)" },
        yn(cfg.run_tests_after_changes),
        yn(cfg.run_build_after_changes),
        if cfg.allow_dependency_install { "allowed" } else { "not allowed" },
        if cfg.allow_git_push { "allowed" } else { "not allowed" },
        if cfg.allow_git_commit { "allowed" } else { "not allowed (leave changes uncommitted)" },
    );
    if !cfg.auto_repair {
        s.push_str(
            "\n- Do not loop on failing checks; stop after the first failure and report it.",
        );
    }
    s
}
