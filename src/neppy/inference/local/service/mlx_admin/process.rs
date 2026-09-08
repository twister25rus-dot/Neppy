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
use super::argv::{build_argv, redact_argv};
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
        clear_marker_at(&mlx_spawn_marker_path(config, &self.id));
    }
}

/// Start the server described by `server`.
///
/// Reclaims a prior orphan for the same id first, so a crashed Neppy cannot
/// leave a model resident and the port occupied.
pub(crate) async fn spawn(config: &Config, server: &MlxServerConfig) -> Result<MlxProcess, String> {
    reclaim_orphan_if_ours(config, &server.id);

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

    let mut command = tokio::process::Command::new(&resolved.path);
    command
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // Kill the server if Neppy dies without running its shutdown hook.
        // The marker covers the cases this cannot.
        .kill_on_drop(true);
    apply_no_window(&mut command);

    log::info!(
        "[mlx] starting server `{}`: {} {}",
        server.id,
        resolved.path.display(),
        redacted_argv.join(" ")
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

    let marker = SpawnMarker::new(pid, &resolved.path);
    if let Err(err) = write_marker_at(&mlx_spawn_marker_path(config, &server.id), &marker) {
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
        child: Some(child),
        logs,
    })
}

/// Kill a process left behind by a previous Neppy that died without stopping
/// its children. Only ever touches a PID we recorded ourselves.
pub(crate) fn reclaim_orphan_if_ours(config: &Config, id: &str) {
    let path = mlx_spawn_marker_path(config, id);
    let Some(marker) = read_marker_at(&path) else {
        return;
    };

    if !pid_is_alive(marker.pid) {
        log::debug!(
            "[mlx] stale spawn marker for `{id}` (pid={} no longer alive); clearing",
            marker.pid
        );
        clear_marker_at(&path);
        return;
    }

    log::info!(
        "[mlx] reclaiming orphaned server `{id}` pid={} from a previous session",
        marker.pid
    );
    super::super::ollama_admin::kill_pid_by_id(marker.pid);
    clear_marker_at(&path);
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
