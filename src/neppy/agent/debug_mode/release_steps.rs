//! The real processes behind "Publish release": the `gh auth status` probe and
//! the detached launcher that runs `scripts/release-neppy.sh`.
//!
//! The launcher leads its own session (`setsid`), so the release survives the
//! app quitting: the script pushes commits, a tag and a GitHub release, and
//! dying halfway through that is worse than finishing. Its stdout and stderr go
//! to a log file (not a pipe) and a sidecar file receives the script's exit
//! code, which is how a later status call learns the outcome without being the
//! process that started it.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;

use super::local_install_steps::build_path;

/// How long `gh auth status` may take before the probe reports "not ready".
const GH_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the script, then records its exit code in `$2` (the sidecar file).
/// `$1` is the version. Passed as argv, so the version is never parsed by a shell.
pub(super) const LAUNCH_SCRIPT: &str = r#"bash scripts/release-neppy.sh "$1"; echo $? > "$2""#;

/// Everything the launcher needs; the spawner adds nothing of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SpawnRequest {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Receives stdout and stderr (appended; the caller truncates it first).
    pub log_path: PathBuf,
}

/// Starts the release launcher detached from this process; returns its pid.
pub(super) trait ReleaseSpawner: Send + Sync {
    fn spawn(&self, req: &SpawnRequest) -> Result<u32, String>;
}

/// Whether `gh` is installed and authenticated.
#[async_trait]
pub(super) trait GhProbe: Send + Sync {
    async fn ready(&self) -> bool;
}

pub(super) struct GhAuthProbe;

#[async_trait]
impl GhProbe for GhAuthProbe {
    async fn ready(&self) -> bool {
        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(["auth", "status"])
            .env("PATH", build_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                log::debug!("[debug_mode][release] gh probe cannot start: {e}");
                return false;
            }
        };
        match tokio::time::timeout(GH_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) => status.success(),
            Ok(Err(e)) => {
                log::debug!("[debug_mode][release] gh probe wait failed: {e}");
                false
            }
            Err(_) => {
                log::debug!("[debug_mode][release] gh probe timed out");
                let _ = child.kill().await;
                false
            }
        }
    }
}

/// Whether `pid` is a live process. `kill(pid, 0)` delivers nothing; EPERM
/// still means "exists" (another user's process).
#[cfg(unix)]
pub(super) fn pid_alive(pid: u32) -> bool {
    // pid 0 and anything that wraps negative would address a process group.
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    // SAFETY: signal 0 performs only the existence/permission check.
    let rc = unsafe { libc::kill(pid as i32, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
pub(super) fn pid_alive(_pid: u32) -> bool {
    false
}

pub(super) struct ShellReleaseSpawner;

impl ReleaseSpawner for ShellReleaseSpawner {
    #[cfg(unix)]
    fn spawn(&self, req: &SpawnRequest) -> Result<u32, String> {
        use std::os::unix::process::CommandExt;
        let (program, args) = req.argv.split_first().ok_or("empty launcher argv")?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&req.log_path)
            .map_err(|e| format!("cannot open the release log: {e}"))?;
        let log2 = log
            .try_clone()
            .map_err(|e| format!("cannot open the release log: {e}"))?;
        let mut cmd = std::process::Command::new(program);
        cmd.args(args)
            .current_dir(&req.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log2));
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        // SAFETY: `setsid` is async-signal-safe and only detaches the child
        // from this process's session and controlling terminal.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot start the release process: {e}"))?;
        let pid = child.id();
        // Reap it when it exits while this app is still alive: an unreaped
        // zombie still answers `kill(pid, 0)` and would look like a live release.
        // After a restart the orphan belongs to init, which reaps it.
        let reaper = std::thread::Builder::new()
            .name("release-reaper".into())
            .spawn(move || {
                let _ = child.wait();
            });
        if let Err(e) = reaper {
            log::warn!("[debug_mode][release] cannot start reaper thread: {e}");
        }
        Ok(pid)
    }

    #[cfg(not(unix))]
    fn spawn(&self, _req: &SpawnRequest) -> Result<u32, String> {
        Err("publishing a release is only supported on macOS".into())
    }
}

/// `release-neppy.sh` path inside a project root.
pub(super) fn script_path(root: &Path) -> PathBuf {
    root.join("scripts/release-neppy.sh")
}
