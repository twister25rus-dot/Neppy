//! Locate the `claude` CLI binary and verify it meets `MIN_CLI_VERSION`.
//!
//! We rely on `claude --version`, which prints a line of the form:
//!   `2.0.4 (Claude Code)`
//! The first whitespace-delimited token is the semver string we compare
//! against [`MIN_CLI_VERSION`].

use std::path::PathBuf;
use std::process::Command;

use super::types::{CliStatus, MIN_CLI_VERSION};

/// Locate the `claude` CLI binary.
///
/// `PATH` alone is not enough, and this is the whole reason the CLI reads as
/// missing in the desktop app while working fine in a terminal. A macOS app
/// launched from Finder or the Dock inherits the minimal launchd `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`), so a `claude` installed by a node version
/// manager (`~/.nvm/versions/node/<v>/bin`), by the native installer
/// (`~/.local/bin`), or by Homebrew is invisible — the same class of bug as
/// #3425 for piper, which `inference::paths` already works around for its own
/// binaries.
///
/// Candidates are tried in order and the first that **works** wins, not the
/// first that exists: a stale install can leave a `claude` that is not runnable
/// here at all — on this author's machine `/usr/local/bin/claude` is a symlink
/// to a Windows `claude.exe` — and picking it would swap "not found" for a
/// spawn failure on every turn. If none runs, the first that exists is returned
/// anyway so [`probe`] can report `Unusable` against a real path instead of
/// claiming nothing is installed.
///
/// Honors `OPENHUMAN_CLAUDE_CLI` so tests and power users can point at one
/// exactly; that path is taken as given and never probed.
pub fn resolve_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("OPENHUMAN_CLAUDE_CLI") {
        let p = PathBuf::from(explicit);
        if p.exists() {
            return Some(p);
        }
    }

    let candidates = candidate_paths();
    let mut first_existing: Option<PathBuf> = None;
    for candidate in candidates {
        if !candidate.is_file() {
            continue;
        }
        if first_existing.is_none() {
            first_existing = Some(candidate.clone());
        }
        if runs(&candidate) {
            return Some(candidate);
        }
        log::debug!(
            "[claude-code][version] {} exists but does not run; trying the next candidate",
            candidate.display()
        );
    }
    first_existing
}

/// Whether this binary answers `--version` with something parseable.
fn runs(path: &std::path::Path) -> bool {
    Command::new(path)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| parse_version(&String::from_utf8_lossy(&out.stdout)).is_some())
        .unwrap_or(false)
}

/// Everywhere a `claude` may live, most specific first.
///
/// `PATH` leads because a user who put it there means it. The rest are the
/// locations an installer uses without touching a GUI app's environment.
fn candidate_paths() -> Vec<PathBuf> {
    candidate_paths_from(
        which_on_path("claude"),
        directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()),
    )
}

/// The candidate list, given what `PATH` produced and where home is.
///
/// Split out from [`candidate_paths`] so the order can be pinned in a test
/// without a real `HOME` or a real `PATH` — the order is the whole behaviour
/// here, and it is not observable from the outside once a binary is picked.
fn candidate_paths_from(path_hit: Option<PathBuf>, home: Option<PathBuf>) -> Vec<PathBuf> {
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let mut out: Vec<PathBuf> = Vec::new();

    if let Some(from_path) = path_hit {
        out.push(from_path);
    }
    if cfg!(windows) {
        return out;
    }

    if let Some(home) = home {
        // The native installer's symlink, and Claude Code's own local install.
        out.push(home.join(".local").join("bin").join(name));
        out.push(home.join(".claude").join("local").join(name));
        // Node version managers put a global npm install under a versioned
        // directory that no GUI app has on its PATH. Newest first, so a current
        // CLI is preferred over one left behind by an older runtime.
        let nvm_versions = home.join(".nvm").join("versions").join("node");
        if let Ok(entries) = std::fs::read_dir(&nvm_versions) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
                .collect();
            versions.sort();
            versions.reverse();
            out.extend(versions.into_iter().map(|v| v.join("bin").join(name)));
        }
        out.push(home.join(".volta").join("bin").join(name));
        out.push(home.join(".bun").join("bin").join(name));
    }

    // Homebrew and the system dirs, last: a broken leftover most often lives in
    // `/usr/local/bin`.
    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        out.push(PathBuf::from(dir).join(name));
    }
    out
}

fn which_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path_var) {
        if cfg!(windows) {
            for ext in &exts {
                let candidate = dir.join(format!("{name}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        } else {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Probe the `claude` CLI and return its status.
pub fn probe() -> CliStatus {
    let Some(path) = resolve_binary() else {
        log::debug!("[claude-code][version] no `claude` binary on PATH");
        return CliStatus::NotInstalled;
    };
    let path_str = path.display().to_string();

    let output = match Command::new(&path).arg("--version").output() {
        Ok(o) => o,
        Err(e) => {
            log::warn!("[claude-code][version] spawn failed path={path_str} err={e}");
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("spawn failed: {e}"),
            };
        }
    };

    if !output.status.success() {
        return CliStatus::Unusable {
            path: path_str,
            reason: format!(
                "non-zero exit {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = match parse_version(&stdout) {
        Some(v) => v,
        None => {
            return CliStatus::Unusable {
                path: path_str,
                reason: format!("could not parse version from: {stdout:?}"),
            }
        }
    };

    if version_lt(&version, MIN_CLI_VERSION) {
        CliStatus::Outdated {
            version,
            min_required: MIN_CLI_VERSION.to_string(),
            path: path_str,
        }
    } else {
        CliStatus::Ok {
            version,
            path: path_str,
        }
    }
}

fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .split_whitespace()
        .next()
        .filter(|tok| tok.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|s| s.to_string())
}

/// Numeric semver compare. Returns true when `a < b`.
/// Pre-release suffixes (`-rc.1`) are stripped before comparison.
fn version_lt(a: &str, b: &str) -> bool {
    let pa = parts(a);
    let pb = parts(b);
    pa < pb
}

fn parts(v: &str) -> (u32, u32, u32) {
    let core = v.split('-').next().unwrap_or(v);
    let mut it = core.split('.').map(|s| s.parse::<u32>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_output() {
        assert_eq!(
            parse_version("2.0.4 (Claude Code)\n").as_deref(),
            Some("2.0.4")
        );
    }

    #[test]
    fn rejects_non_numeric_prefix() {
        assert_eq!(parse_version("claude version 2.0.4"), None);
    }

    #[test]
    fn version_compare() {
        assert!(version_lt("1.9.9", "2.0.0"));
        assert!(version_lt("2.0.0", "2.0.1"));
        assert!(!version_lt("2.0.0", "2.0.0"));
        assert!(!version_lt("2.1.0", "2.0.9"));
    }

    #[test]
    fn a_path_hit_leads_but_is_not_the_only_candidate() {
        // The GUI-launched app is the case that matters: with nothing on PATH
        // there must still be somewhere to look, or the CLI reads as missing
        // while working fine in a terminal.
        let home = PathBuf::from("/home/someone");
        let with_path =
            candidate_paths_from(Some(PathBuf::from("/w/bin/claude")), Some(home.clone()));
        assert_eq!(with_path.first(), Some(&PathBuf::from("/w/bin/claude")));

        let without_path = candidate_paths_from(None, Some(home.clone()));
        assert!(
            !without_path.is_empty(),
            "an empty PATH must not mean an empty candidate list"
        );
        assert!(without_path.contains(&home.join(".local").join("bin").join("claude")));
        assert!(without_path.contains(&home.join(".claude").join("local").join("claude")));
    }

    #[test]
    fn homebrew_and_usr_local_come_after_the_user_installs() {
        // `/usr/local/bin/claude` is where a stale or wrong-platform install
        // tends to linger, so it must never outrank a user's own.
        let home = PathBuf::from("/home/someone");
        let candidates = candidate_paths_from(None, Some(home.clone()));

        let local_bin = candidates
            .iter()
            .position(|c| *c == home.join(".local").join("bin").join("claude"))
            .expect("~/.local/bin is a candidate");
        let usr_local = candidates
            .iter()
            .position(|c| *c == PathBuf::from("/usr/local/bin/claude"))
            .expect("/usr/local/bin is a candidate");

        assert!(local_bin < usr_local, "a user install must be preferred");
    }

    #[test]
    fn node_version_manager_installs_are_found_newest_first() {
        // An npm global install lands under a versioned directory no GUI app
        // has on its PATH. Two runtimes installed, the newer one wins.
        let home = tempfile::tempdir().expect("tempdir");
        let versions = home.path().join(".nvm").join("versions").join("node");
        for version in ["v20.11.0", "v24.16.0"] {
            std::fs::create_dir_all(versions.join(version).join("bin")).expect("mkdir");
        }

        let candidates = candidate_paths_from(None, Some(home.path().to_path_buf()));
        let nvm: Vec<&PathBuf> = candidates
            .iter()
            .filter(|c| c.starts_with(&versions))
            .collect();

        assert_eq!(nvm.len(), 2);
        assert!(
            nvm[0].to_string_lossy().contains("v24.16.0"),
            "newest runtime first, got {nvm:?}"
        );
    }

    #[test]
    fn version_compare_strips_prerelease() {
        assert!(!version_lt("2.0.0-rc.1", "2.0.0"));
    }
}
