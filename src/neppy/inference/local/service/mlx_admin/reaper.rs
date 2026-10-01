//! Making sure no MLX worker outlives the core that spawned it.
//!
//! A worker holds the model weights, so an orphan is several GiB of resident
//! memory that nothing will ever free. Three things used to leave one behind:
//!
//! - `serve` got SIGTERM and exited without anyone calling the pool's
//!   `shutdown_all` (it had no caller). `kill_on_drop` did not help: the pool
//!   lives in a process-wide singleton, which is never dropped.
//! - the desktop app aborts the embedded server task on quit, so the code
//!   after `axum::serve` never ran;
//! - a hard crash (`kill -9`, power loss) runs nothing at all.
//!
//! This module is the one place that stops workers, in three entry points that
//! share [`terminate_blocking`]:
//!
//! - [`stop_pids`] for a pool that knows its own children;
//! - [`shutdown_tracked_workers`] / [`shutdown_tracked_workers_blocking`] for
//!   "everything this process spawned", driven by a process-wide registry so a
//!   caller that has no pool or config in reach (the desktop teardown) can
//!   still do it;
//! - [`reap_stale_workers`] on the next boot, for the crash case.
//!
//! Stopping is graceful first (SIGTERM, bounded wait) then SIGKILL, and every
//! wait is bounded so shutdown cannot hang on a wedged worker.
//!
//! **A one-shot CLI process is deliberately not a supervisor.** `neppy-core mlx
//! start` spawns a server and exits, and that server is meant to outlive it
//! (`mlx stop` reclaims it through the marker). So stale-reaping only touches
//! markers written by a *supervised host* (`serve` / the embedded core), which
//! declares itself with [`declare_supervised_host`]. A marker from an older
//! build has no such flag and is left to the existing same-id reclaim in
//! `process::spawn`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::neppy::config::Config;
use crate::neppy::inference::paths::mlx_spawn_marker_path;

use super::super::spawn_marker::{clear_marker_at, read_marker_at};

/// How long a worker gets to exit after SIGTERM before it is killed.
pub(crate) const SHUTDOWN_GRACE: Duration = Duration::from_millis(1500);

/// Hard ceiling for the async entry point. Covers the grace period, the wait
/// after SIGKILL, and a slow blocking-pool thread.
pub(crate) const SHUTDOWN_BUDGET: Duration = Duration::from_secs(3);

/// Poll interval while waiting for a process to exit.
const POLL: Duration = Duration::from_millis(25);

/// Wait after SIGKILL for the kernel to tear the process down.
const KILL_WAIT: Duration = Duration::from_millis(500);

static SUPERVISED_HOST: AtomicBool = AtomicBool::new(false);

/// Declare that this process is a long-lived host that stops its own workers
/// (`serve`, the embedded desktop core). Workers it spawns afterwards are
/// recorded as `supervised`, which is what licenses reaping them later.
pub(crate) fn declare_supervised_host() {
    SUPERVISED_HOST.store(true, Ordering::SeqCst);
}

pub(crate) fn is_supervised_host() -> bool {
    SUPERVISED_HOST.load(Ordering::SeqCst)
}

/// A worker this process spawned.
#[derive(Debug, Clone)]
pub(crate) struct Tracked {
    pub(crate) pid: u32,
    pub(crate) marker_path: PathBuf,
    /// File name of the binary, used to confirm a PID still names our worker.
    pub(crate) binary_name: Option<String>,
}

static LIVE: Lazy<Mutex<HashMap<u32, Tracked>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Record a freshly spawned worker.
pub(crate) fn track(pid: u32, marker_path: &Path, binary_path: &Path) {
    let binary_name = binary_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty());
    log::debug!("[mlx:reaper] tracking worker pid={pid}");
    LIVE.lock().insert(
        pid,
        Tracked {
            pid,
            marker_path: marker_path.to_path_buf(),
            binary_name,
        },
    );
}

/// Stop tracking a worker that was stopped by the normal path.
pub(crate) fn forget(pid: u32) {
    if LIVE.lock().remove(&pid).is_some() {
        log::debug!("[mlx:reaper] no longer tracking pid={pid}");
    }
}

#[cfg(test)]
pub(crate) fn tracked_count() -> usize {
    LIVE.lock().len()
}

/// Whether `pid` is a live, non-zombie process.
///
/// A child that has exited but not been reaped still answers `kill(pid, 0)`,
/// so liveness has to come from the process status, or a worker that died on
/// SIGTERM would look alive for the whole grace period.
pub(crate) fn pid_running(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessStatus, ProcessesToUpdate, System};
    let target = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    sys.process(target)
        .is_some_and(|p| !matches!(p.status(), ProcessStatus::Zombie | ProcessStatus::Dead))
}

/// Whether the command line of `pid` names `binary_name`.
///
/// Guards against PID reuse: a marker or registry entry can outlive its
/// process, and the number may since belong to something unrelated. When the
/// command line cannot be read (some platforms, other users' processes) the
/// answer is `unknown`, which the caller decides how to treat.
fn cmdline_names(pid: u32, binary_name: &str) -> Option<bool> {
    use sysinfo::{Pid, ProcessesToUpdate, System, UpdateKind};
    let target = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let process = sys.process(target)?;
    let mut seen_any = false;
    let mut hit = false;
    for arg in process.cmd() {
        seen_any = true;
        if arg.to_string_lossy().contains(binary_name) {
            hit = true;
        }
    }
    if let Some(exe) = process.exe() {
        seen_any = true;
        if exe.to_string_lossy().contains(binary_name) {
            hit = true;
        }
    }
    seen_any.then_some(hit)
}

#[cfg(unix)]
fn signal(pid: u32, sig: i32) {
    // SAFETY: plain kill(2) on a pid we verified; no memory is touched.
    let rc = unsafe { libc::kill(pid as i32, sig) };
    if rc != 0 {
        log::debug!(
            "[mlx:reaper] kill({pid}, {sig}) failed: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn request_exit(pid: u32) {
    #[cfg(unix)]
    signal(pid, libc::SIGTERM);
    #[cfg(not(unix))]
    super::super::ollama_admin::kill_pid_by_id(pid);
}

fn force_exit(pid: u32) {
    #[cfg(unix)]
    signal(pid, libc::SIGKILL);
    #[cfg(not(unix))]
    super::super::ollama_admin::kill_pid_by_id(pid);
}

fn wait_until_gone(pids: &[u32], budget: Duration) -> Vec<u32> {
    let deadline = Instant::now() + budget;
    let mut alive: Vec<u32> = pids.iter().copied().filter(|p| pid_running(*p)).collect();
    while !alive.is_empty() && Instant::now() < deadline {
        std::thread::sleep(POLL);
        alive.retain(|p| pid_running(*p));
    }
    alive
}

/// Stop the given workers: SIGTERM, wait up to `grace`, SIGKILL the rest.
///
/// Blocking, and bounded by `grace + KILL_WAIT`. A pid that no longer matches
/// its binary is skipped (and its marker cleared) rather than signalled.
/// Returns how many processes were actually stopped.
pub(crate) fn terminate_blocking(workers: &[Tracked], grace: Duration) -> usize {
    let mut targets: Vec<&Tracked> = Vec::new();
    for worker in workers {
        forget(worker.pid);
        if !pid_running(worker.pid) {
            log::debug!("[mlx:reaper] pid={} already gone", worker.pid);
            clear_marker_at(&worker.marker_path);
            continue;
        }
        if let Some(name) = &worker.binary_name {
            if cmdline_names(worker.pid, name) == Some(false) {
                log::warn!(
                    "[mlx:reaper] pid={} no longer runs `{name}`; not signalling a reused pid",
                    worker.pid
                );
                clear_marker_at(&worker.marker_path);
                continue;
            }
        }
        targets.push(worker);
    }
    if targets.is_empty() {
        return 0;
    }

    let pids: Vec<u32> = targets.iter().map(|w| w.pid).collect();
    log::info!("[mlx:reaper] stopping workers {pids:?} (grace {grace:?})");
    for pid in &pids {
        request_exit(*pid);
    }
    let stubborn = wait_until_gone(&pids, grace);
    if !stubborn.is_empty() {
        log::warn!("[mlx:reaper] workers {stubborn:?} ignored SIGTERM; killing");
        for pid in &stubborn {
            force_exit(*pid);
        }
        let left = wait_until_gone(&stubborn, KILL_WAIT);
        if !left.is_empty() {
            log::error!("[mlx:reaper] workers {left:?} still alive after SIGKILL");
        }
    }
    for worker in &targets {
        clear_marker_at(&worker.marker_path);
    }
    pids.len()
}

/// Stop exactly these workers without blocking the async runtime.
pub(crate) async fn stop_pids(workers: Vec<Tracked>, grace: Duration) -> usize {
    if workers.is_empty() {
        return 0;
    }
    let task = tokio::task::spawn_blocking(move || terminate_blocking(&workers, grace));
    match tokio::time::timeout(grace + KILL_WAIT + Duration::from_secs(1), task).await {
        Ok(Ok(n)) => n,
        Ok(Err(err)) => {
            log::warn!("[mlx:reaper] stop task failed: {err}");
            0
        }
        Err(_) => {
            log::warn!("[mlx:reaper] stop exceeded its budget; proceeding with shutdown");
            0
        }
    }
}

fn drain_registry() -> Vec<Tracked> {
    LIVE.lock().drain().map(|(_, v)| v).collect()
}

/// Stop every worker this process has spawned. Blocking; safe to call from a
/// thread with no runtime (the desktop's synchronous teardown). Idempotent.
pub(crate) fn shutdown_tracked_workers_blocking(grace: Duration) -> usize {
    let workers = drain_registry();
    if workers.is_empty() {
        return 0;
    }
    log::info!(
        "[mlx:reaper] shutdown: {} tracked worker(s) to stop",
        workers.len()
    );
    terminate_blocking(&workers, grace)
}

/// Async form of [`shutdown_tracked_workers_blocking`], bounded by
/// [`SHUTDOWN_BUDGET`].
pub(crate) async fn shutdown_tracked_workers(grace: Duration) -> usize {
    stop_pids(drain_registry(), grace).await
}

/// Reap workers left behind by a supervised host that died without cleaning
/// up. Returns how many were stopped.
///
/// A marker qualifies only when all of these hold: it was written by a
/// supervised host, that host's PID is gone (or is not us), the worker PID is
/// alive, and the worker's command line still names the recorded binary.
/// Anything else is left alone: a server another live core owns, a CLI-started
/// server, or a PID that has been recycled.
pub(crate) fn reap_stale_workers(config: &Config) -> usize {
    let probe = mlx_spawn_marker_path(config, "probe");
    let Some(dir) = probe.parent() else {
        return 0;
    };
    reap_stale_in(dir, std::process::id())
}

pub(crate) fn reap_stale_in(dir: &Path, our_pid: u32) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut stale: Vec<Tracked> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_mlx_marker = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("mlx-") && n.ends_with(".spawn"));
        if !is_mlx_marker {
            continue;
        }
        let Some(marker) = read_marker_at(&path) else {
            continue;
        };
        if !marker.supervised {
            log::debug!(
                "[mlx:reaper] {} was not written by a supervised host; leaving it",
                path.display()
            );
            continue;
        }
        if marker.neppy_pid == our_pid {
            continue;
        }
        if pid_running(marker.neppy_pid) {
            log::debug!(
                "[mlx:reaper] {} belongs to live core pid={}; leaving it",
                path.display(),
                marker.neppy_pid
            );
            continue;
        }
        if !pid_running(marker.pid) {
            log::debug!(
                "[mlx:reaper] stale marker {} (worker pid={} gone); clearing",
                path.display(),
                marker.pid
            );
            clear_marker_at(&path);
            continue;
        }
        let binary_name = Path::new(&marker.binary_path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty());
        // Unlike our own children, a recorded pid has no handle behind it, so
        // an unreadable command line is not enough evidence to kill.
        let confirmed = match &binary_name {
            Some(name) => cmdline_names(marker.pid, name) == Some(true),
            None => false,
        };
        if !confirmed {
            log::warn!(
                "[mlx:reaper] marker {} names pid={} but it does not look like `{}`; clearing marker only",
                path.display(),
                marker.pid,
                marker.binary_path
            );
            clear_marker_at(&path);
            continue;
        }
        log::warn!(
            "[mlx:reaper] reaping worker pid={} orphaned by dead core pid={}",
            marker.pid,
            marker.neppy_pid
        );
        stale.push(Tracked {
            pid: marker.pid,
            marker_path: path,
            binary_name,
        });
    }
    terminate_blocking(&stale, SHUTDOWN_GRACE)
}

#[cfg(test)]
#[path = "reaper_tests.rs"]
mod tests;
