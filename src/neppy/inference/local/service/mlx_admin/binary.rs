//! Locating the MLX server executables.
//!
//! Resolution order, first hit wins:
//!
//! 1. `mlx.bin_dir` from config, when set
//! 2. `MLX_VLM_BIN` / `MLX_LM_BIN` environment overrides
//! 3. `PATH`
//! 4. `~/.local/bin`, where `uv tool install` places entry points
//!
//! A miss is a distinct, actionable state rather than a generic failure: the
//! error names the exact install command, because "MLX unavailable" with no
//! next step is the least useful thing this module could say.

use std::path::{Path, PathBuf};

use crate::neppy::config::schema::MlxServerConfig;
use crate::neppy::config::Config;

/// Where a resolved binary came from, for diagnostics and the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BinarySource {
    ConfiguredDir,
    EnvOverride,
    Path,
    UvToolDir,
}

impl BinarySource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ConfiguredDir => "mlx.bin_dir",
            Self::EnvOverride => "environment override",
            Self::Path => "PATH",
            Self::UvToolDir => "~/.local/bin",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedBinary {
    pub(crate) path: PathBuf,
    pub(crate) source: BinarySource,
}

/// Environment override variable for a server kind.
fn env_var_for(server: &MlxServerConfig) -> &'static str {
    if server.is_vlm() {
        "MLX_VLM_BIN"
    } else {
        "MLX_LM_BIN"
    }
}

/// The package that provides a server kind, named in the install hint.
fn package_for(server: &MlxServerConfig) -> &'static str {
    if server.is_vlm() {
        "mlx-vlm"
    } else {
        "mlx-lm"
    }
}

/// Resolve the executable for `server`, or explain how to install it.
pub(crate) fn resolve_binary(
    config: &Config,
    server: &MlxServerConfig,
) -> Result<ResolvedBinary, String> {
    let name = server.binary_name();

    let configured = config.mlx.bin_dir.trim();
    if !configured.is_empty() {
        let candidate = expand_home(configured).join(name);
        if is_executable(&candidate) {
            return Ok(ResolvedBinary {
                path: candidate,
                source: BinarySource::ConfiguredDir,
            });
        }
        // A configured directory that does not hold the binary is a user
        // mistake worth naming, but not fatal — keep looking.
        log::warn!(
            "[mlx] mlx.bin_dir is set to `{configured}` but does not contain {name}; \
             falling through to PATH"
        );
    }

    let env_var = env_var_for(server);
    if let Some(from_env) = std::env::var(env_var).ok().filter(|v| !v.trim().is_empty()) {
        let candidate = expand_home(from_env.trim());
        if is_executable(&candidate) {
            return Ok(ResolvedBinary {
                path: candidate,
                source: BinarySource::EnvOverride,
            });
        }
        log::warn!("[mlx] {env_var} points at `{from_env}`, which is not executable");
    }

    if let Some(found) = search_path(name) {
        return Ok(ResolvedBinary {
            path: found,
            source: BinarySource::Path,
        });
    }

    let uv_bin = uv_tool_bin_dir().join(name);
    if is_executable(&uv_bin) {
        return Ok(ResolvedBinary {
            path: uv_bin,
            source: BinarySource::UvToolDir,
        });
    }

    Err(format!(
        "{name} was not found. Install it with `uv tool install {}`, \
         or set mlx.bin_dir to the directory that holds it.",
        package_for(server)
    ))
}

/// Probe a resolved binary by running `--help`, which loads no model weights.
///
/// Used at start to turn "the file exists" into "the file runs", so a broken
/// install fails with a clear message instead of a spawn that dies seconds
/// later with an opaque exit code.
pub(crate) async fn probe_binary(path: &Path) -> Result<(), String> {
    let output = tokio::process::Command::new(path)
        .arg("--help")
        .output()
        .await
        .map_err(|err| format!("could not execute {}: {err}", path.display()))?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail: String = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
    Err(format!(
        "{} exited {} when probed: {tail}",
        path.display(),
        output
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "by signal".to_string())
    ))
}

fn search_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

fn uv_tool_bin_dir() -> PathBuf {
    home_dir().join(".local").join("bin")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Expand a leading `~` so config values can stay portable.
fn expand_home(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if raw == "~" {
        return home_dir();
    }
    PathBuf::from(raw)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
