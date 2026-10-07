//! Failed-launch detection for Debug Mode recovery (spec section 37).
//!
//! At startup the shell writes `launch-pending.json` into the Neppy data dir
//! (the same dir the logs live in). Once the core reports ready it deletes
//! that file. A marker that is *still there* at the next startup, left by a
//! process that is no longer alive, means the previous launch never reached
//! ready: it is logged and copied to `last-failed-launch.json`, which
//! `scripts/neppy-recover.sh status` reads next to the pending marker.
//!
//! The logic is pure over a directory so it is unit-testable; [`begin`] and
//! [`mark_ready`] bind it to the real data dir. Every failure here is logged
//! and swallowed: bookkeeping must never stop the app from starting.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const PENDING_FILE: &str = "launch-pending.json";
pub(crate) const FAILED_FILE: &str = "last-failed-launch.json";

/// What a launch writes about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LaunchMarker {
    pub pid: u32,
    pub version: String,
    pub started_at: String,
}

/// A previous launch that never reached ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FailedLaunch {
    pub pid: u32,
    pub version: String,
    pub started_at: String,
    pub detected_at: String,
}

fn write_atomic(dir: &Path, name: &str, value: &impl Serialize) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let go = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&data)?;
        f.sync_all()?;
        fs::rename(&tmp, dir.join(name))
    };
    go().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("write {name}: {e}")
    })
}

fn read_marker(dir: &Path) -> Option<LaunchMarker> {
    let bytes = fs::read(dir.join(PENDING_FILE)).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(m) => Some(m),
        Err(e) => {
            // An unreadable marker is still evidence that a launch was
            // pending; keep the pid out of it.
            log::warn!("[launch_marker] unreadable {PENDING_FILE} ({e}); treating as failed");
            Some(LaunchMarker {
                pid: 0,
                version: "unknown".into(),
                started_at: "unknown".into(),
            })
        }
    }
}

/// Records this launch in `dir`. If a stale marker from a dead process is
/// present, records and returns the failed launch it describes. `is_alive`
/// keeps a concurrently starting second instance from being mistaken for a
/// crash.
pub(crate) fn begin_in(
    dir: &Path,
    pid: u32,
    version: &str,
    now: &str,
    is_alive: &dyn Fn(u32) -> bool,
) -> Option<FailedLaunch> {
    let mut failed = None;
    if let Some(stale) = read_marker(dir) {
        let other_instance_running = stale.pid != 0 && stale.pid != pid && is_alive(stale.pid);
        if other_instance_running {
            log::info!(
                "[launch_marker] pending marker belongs to running pid={}; not a failed launch",
                stale.pid
            );
        } else {
            log::warn!(
                "[launch_marker] previous launch did not reach ready (pid={} version={} started_at={})",
                stale.pid,
                stale.version,
                stale.started_at
            );
            let f = FailedLaunch {
                pid: stale.pid,
                version: stale.version,
                started_at: stale.started_at,
                detected_at: now.to_string(),
            };
            if let Err(e) = write_atomic(dir, FAILED_FILE, &f) {
                log::warn!("[launch_marker] cannot record failed launch: {e}");
            }
            failed = Some(f);
        }
    }
    let marker = LaunchMarker {
        pid,
        version: version.to_string(),
        started_at: now.to_string(),
    };
    match write_atomic(dir, PENDING_FILE, &marker) {
        Ok(()) => log::debug!("[launch_marker] wrote {PENDING_FILE} pid={pid}"),
        Err(e) => log::warn!("[launch_marker] cannot write pending marker: {e}"),
    }
    failed
}

/// Clears the pending marker. `true` when one was removed.
pub(crate) fn mark_ready_in(dir: &Path) -> bool {
    match fs::remove_file(dir.join(PENDING_FILE)) {
        Ok(()) => {
            log::debug!("[launch_marker] cleared {PENDING_FILE}");
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            log::warn!("[launch_marker] cannot clear {PENDING_FILE}: {e}");
            false
        }
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    matches!(
        kill(Pid::from_raw(pid as i32), None),
        Ok(()) | Err(Errno::EPERM)
    )
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    false
}

fn data_dir() -> PathBuf {
    crate::file_logging::resolve_data_dir()
}

/// Startup hook: write the pending marker, reporting a failed previous launch.
pub(crate) fn begin() {
    let now = chrono::Utc::now().to_rfc3339();
    begin_in(
        &data_dir(),
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        &now,
        &process_alive,
    );
}

/// Core-ready hook: the launch succeeded, so the marker is removed.
pub(crate) fn mark_ready() {
    // Unit tests drive `ensure_running`; they must not touch the real data dir.
    if cfg!(test) {
        return;
    }
    mark_ready_in(&data_dir());
}

#[cfg(test)]
#[path = "launch_marker_tests.rs"]
mod tests;
