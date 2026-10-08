//! Spawning, supervising and stopping one MLX server process.
//!
//! Ownership rules mirror `ollama_admin::server`: Neppy kills only children it
//! started. A spawn marker records the PID so a process orphaned by a crash
//! (shutdown hook never ran) can be reclaimed on the next start instead of
//! leaking a resident 16 GB model.
//!
//! Unlike Ollama there is no shared daemon to adopt — each block is a private
//! child — so the marker exists purely for orphan reclamation.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::neppy::config::schema::MlxServerConfig;
use crate::neppy::config::Config;
use crate::neppy::inference::local::process_util::apply_no_window;
use crate::neppy::inference::paths::mlx_spawn_marker_path;

use super::super::spawn_marker::{
    clear_marker_at, pid_is_alive, read_marker_at, write_marker_at,
    OllamaSpawnMarker as SpawnMarker,
};
use super::argv::{build_argv, redact_argv, spawn_env};
use super::binary::{probe_binary, resolve_binary};

/// Lines of server output retained per process. Bounded so a chatty
/// `--log-level DEBUG` cannot grow without limit; the UI tails this.
const LOG_CAPACITY: usize = 500;

/// A running MLX server owned by this process.
pub(crate) struct MlxProcess {
    pub(crate) id: String,
    pub(crate) port: u16,
    pub(crate) pid: u32,
    pub(crate) binary_path: PathBuf,
    /// The argv used to start it, with any bearer token redacted. Shown in the
    /// UI so "what exactly did you run?" is answerable without guessing.
    pub(crate) redacted_argv: Vec<String>,
    /// Where this process's spawn marker lives, so a stop that has no
    /// `Config` in reach (shutdown) can still clear it.
    marker_path: PathBuf,
    child: Option<tokio::process::Child>,
    logs: Arc<Mutex<VecDeque<String>>>,
}

impl MlxProcess {
    /// Most recent `limit` lines of combined stdout/stderr, oldest first.
    pub(crate) fn logs_tail(&self, limit: usize) -> Vec<String> {
        let buffer = self.logs.lock();
        let skip = buffer.len().saturating_sub(limit);
        buffer.iter().skip(skip).cloned().collect()
    }

    /// This process as the reaper tracks it.
    pub(crate) fn tracked(&self) -> super::reaper::Tracked {
        super::reaper::Tracked {
            pid: self.pid,
            marker_path: self.marker_path.clone(),
            binary_name: self
                .binary_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|n| !n.is_empty()),
        }
    }

    /// Whether the child has exited, and with what status.
    ///
    /// Non-blocking: `try_wait` reaps without waiting, so this is safe to call
    /// from a health poll.
    pub(crate) fn exit_status(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(match status.code() {
                Some(code) => format!("exited with code {code}"),
                None => "terminated by signal".to_string(),
            }),
            Ok(None) => None,
            Err(err) => Some(format!("could not read exit status: {err}")),
        }
    }

    /// Stop the server and clear its marker.
    pub(crate) async fn stop(&mut self, config: &Config) {
        if let Some(mut child) = self.child.take() {
            if let Err(err) = child.kill().await {
                log::warn!(
                    "[mlx] failed to kill server `{}` pid={}: {err}",
                    self.id,
                    self.pid
                );
            } else {
                log::info!("[mlx] stopped server `{}` pid={}", self.id, self.pid);
            }
        }
        super::reaper::forget(self.pid);
        clear_marker_at(&mlx_spawn_marker_path(config, &self.id));
    }
}

#[cfg(test)]
impl MlxProcess {
    /// Wrap an arbitrary child, for pool tests that need a real process
    /// without an MLX binary.
    pub(crate) fn from_child_for_test(id: &str, port: u16, child: tokio::process::Child) -> Self {
        Self {
            id: id.to_string(),
            port,
            pid: child.id().unwrap_or(0),
            binary_path: PathBuf::new(),
            redacted_argv: Vec::new(),
            marker_path: PathBuf::new(),
            child: Some(child),
            logs: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

/// Start the server described by `server`.
///
/// Reclaims a prior orphan for the same id first, so a crashed Neppy cannot
/// leave a model resident and the port occupied.
pub(crate) async fn spawn(config: &Config, server: &MlxServerConfig) -> Result<MlxProcess, String> {
    // Only a worker a dead supervised core left behind is reclaimed here; a
    // server the user (or a live core, or a one-shot CLI) started is not ours.
    let _ = reclaim_stale_supervised(config, &server.id);
    // Limits the assistant relies on, applied to this launch only. The stored
    // block is never rewritten.
    let server = &super::argv::apply_spawn_limits(config, server);

    let resolved = resolve_binary(config, server)?;
    probe_binary(&resolved.path).await?;
    log::debug!(
        "[mlx] resolved {} via {} at {}",
        server.binary_name(),
        resolved.source.as_str(),
        resolved.path.display()
    );

    let port = assign_port(server.port)?;
    let args = build_argv(server, port);
    let redacted_argv = redact_argv(&args);

    let env = spawn_env(server, |name| std::env::var(name).ok());

    let mut command = tokio::process::Command::new(&resolved.path);
    command
        .args(&args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Kill the server if Neppy dies without running its shutdown hook.
        // The marker covers the cases this cannot.
        .kill_on_drop(true);
    apply_no_window(&mut command);

    log::info!(
        "[mlx] starting server `{}`: {} {} env=[{}]",
        server.id,
        resolved.path.display(),
        redacted_argv.join(" "),
        env.iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    let mut child = command
        .spawn()
        .map_err(|err| format!("could not start {}: {err}", resolved.path.display()))?;

    let pid = child
        .id()
        .ok_or_else(|| "server exited before a pid could be read".to_string())?;

    let logs = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)));
    if let Some(stdout) = child.stdout.take() {
        pump_output(stdout, Arc::clone(&logs), server.id.clone(), "out");
    }
    if let Some(stderr) = child.stderr.take() {
        pump_output(stderr, Arc::clone(&logs), server.id.clone(), "err");
    }

    let marker_path = mlx_spawn_marker_path(config, &server.id);
    let mut marker = SpawnMarker::new(pid, &resolved.path);
    marker.supervised = super::reaper::is_supervised_host();
    // From here on shutdown can find this worker without the pool or a config.
    super::reaper::track(pid, &marker_path, &resolved.path);
    if let Err(err) = write_marker_at(&marker_path, &marker) {
        // Not fatal: the process is running and usable. It just means a crash
        // before shutdown would leave an orphan we cannot later identify.
        log::warn!(
            "[mlx] could not write spawn marker for `{}`: {err}",
            server.id
        );
    }

    Ok(MlxProcess {
        id: server.id.clone(),
        port,
        pid,
        binary_path: resolved.path,
        redacted_argv,
        marker_path,
        child: Some(child),
        logs,
    })
}

/// Remove the spawn marker for `id` without touching the process it names.
/// For a server that is known to be gone (it exited, or failed to start): the
/// recorded PID may have been recycled since, so it must not be signalled.
pub(crate) fn clear_marker_only(config: &Config, id: &str) {
    let path = mlx_spawn_marker_path(config, id);
    if let Some(marker) = read_marker_at(&path) {
        super::reaper::forget(marker.pid);
    }
    clear_marker_at(&path);
}

fn marker_binary_name(marker: &SpawnMarker) -> Option<String> {
    std::path::Path::new(&marker.binary_path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
}

/// Reclaim the worker a *dead supervised core* left behind for `id`, so a crash
/// cannot leave a model resident and the port taken. Everything else is left
/// alone and its marker kept or cleared as appropriate:
///
/// - a marker from a one-shot CLI start (`supervised = false`) is meant to
///   outlive that CLI;
/// - a marker whose owning core is still running belongs to that core;
/// - a recorded PID whose command line does not name the recorded binary has
///   been recycled, so only the marker is dropped.
///
/// Returns whether a live process was killed.
pub(crate) fn reclaim_stale_supervised(config: &Config, id: &str) -> bool {
    let path = mlx_spawn_marker_path(config, id);
    let Some(marker) = read_marker_at(&path) else {
        return false;
    };
    reclaim_stale_marker(&path, &marker, std::process::id(), id)
}

pub(crate) fn reclaim_stale_marker(
    path: &std::path::Path,
    marker: &SpawnMarker,
    our_pid: u32,
    id: &str,
) -> bool {
    if !super::reaper::pid_running(marker.pid) {
        log::debug!(
            "[mlx] stale spawn marker for `{id}` (pid={} no longer running); clearing",
            marker.pid
        );
        super::reaper::forget(marker.pid);
        clear_marker_at(path);
        return false;
    }
    if !marker.supervised {
        log::info!(
            "[mlx] `{id}` pid={} was started by a one-shot CLI, not a supervised core; leaving it",
            marker.pid
        );
        return false;
    }
    if marker.neppy_pid != our_pid && super::reaper::pid_running(marker.neppy_pid) {
        log::info!(
            "[mlx] `{id}` pid={} belongs to the live core pid={}; leaving it",
            marker.pid,
            marker.neppy_pid
        );
        return false;
    }
    let confirmed = marker_binary_name(marker)
        .is_some_and(|name| super::reaper::cmdline_names(marker.pid, &name) == Some(true));
    if !confirmed {
        log::warn!(
            "[mlx] marker for `{id}` names pid={} but it does not look like `{}`; clearing the marker only",
            marker.pid,
            marker.binary_path
        );
        clear_marker_at(path);
        return false;
    }
    log::info!(
        "[mlx] reclaiming server `{id}` pid={} orphaned by core pid={}",
        marker.pid,
        marker.neppy_pid
    );
    super::super::ollama_admin::kill_pid_by_id(marker.pid);
    super::reaper::forget(marker.pid);
    clear_marker_at(path);
    true
}

/// Kill a process recorded for `id` that this process does not hold a handle
/// to, because the user asked for that server to be stopped.
///
/// Returns whether a live process was actually killed, so callers can report
/// honestly instead of claiming a stop that did not happen.
///
/// Only the explicit-stop path uses this (`MlxPool::stop`): the CLI is a fresh
/// process per invocation and never shares a pool with the invocation that
/// spawned the server. Automatic callers must not: use
/// [`reclaim_stale_supervised`] to clean up after a dead core and
/// [`clear_marker_only`] for a server known to be gone. A recorded PID that is
/// readable and does not look like the recorded binary is treated as recycled
/// and is not killed.
pub(crate) fn reclaim_orphan_if_ours(config: &Config, id: &str) -> bool {
    let path = mlx_spawn_marker_path(config, id);
    let Some(marker) = read_marker_at(&path) else {
        return false;
    };

    if !pid_is_alive(marker.pid) {
        log::debug!(
            "[mlx] stale spawn marker for `{id}` (pid={} no longer alive); clearing",
            marker.pid
        );
        super::reaper::forget(marker.pid);
        clear_marker_at(&path);
        return false;
    }
    if let Some(name) = marker_binary_name(&marker) {
        if super::reaper::cmdline_names(marker.pid, &name) == Some(false) {
            log::warn!(
                "[mlx] marker for `{id}` names pid={} but it does not look like `{name}`; clearing the marker only",
                marker.pid
            );
            super::reaper::forget(marker.pid);
            clear_marker_at(&path);
            return false;
        }
    }

    log::info!(
        "[mlx] reclaiming server `{id}` pid={} recorded by an earlier process",
        marker.pid
    );
    super::super::ollama_admin::kill_pid_by_id(marker.pid);
    super::reaper::forget(marker.pid);
    clear_marker_at(&path);
    true
}

/// Choose a port: honour an explicit one, otherwise ask the OS for a free one.
///
/// Binding to port 0 and reading back the assignment races with any other
/// process that might claim it in the gap, but the window is small and the
/// alternative — a fixed default that collides with an existing server — is
/// the failure users actually hit.
fn assign_port(configured: u16) -> Result<u16, String> {
    if configured != 0 {
        return Ok(configured);
    }
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|err| format!("could not reserve a port for the MLX server: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("could not read the reserved port: {err}"))?
        .port();
    drop(listener);
    Ok(port)
}

/// Forward a child stream into the ring buffer, tagging which stream it came
/// from so a crash cause is readable in the UI without a separate log file.
fn pump_output<R>(stream: R, logs: Arc<Mutex<VecDeque<String>>>, id: String, tag: &'static str)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut buffer = logs.lock();
            if buffer.len() == LOG_CAPACITY {
                buffer.pop_front();
            }
            buffer.push_back(format!("[{tag}] {line}"));
        }
        log::debug!("[mlx] `{id}` {tag} stream closed");
    });
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
