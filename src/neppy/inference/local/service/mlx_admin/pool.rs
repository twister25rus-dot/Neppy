//! The supervisor: reconciles running processes against configured blocks.
//!
//! One `[[mlx.server]]` block is the unit. The pool owns every process it
//! started, keyed by block id, and is the only thing that knows a block's
//! *resolved* port — which matters because `port = 0` means "assign at start",
//! so `local::mlx::mlx_base_url` can only see blocks with a fixed port. Once a
//! block is running, `base_url_for` is the authoritative answer.
//!
//! Locking is `tokio::sync::Mutex` rather than `parking_lot`, because starting
//! a server awaits both the spawn and the readiness poll.

use std::collections::HashMap;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Mutex;

use crate::neppy::config::schema::MlxServerConfig;
use crate::neppy::config::Config;

use super::health::{classify, probe_liveness, probe_models, MlxServerState};
use super::memory::{admit, budget_gib, resident_gib, Admission};
use super::process::{spawn, MlxProcess};

/// Grace period for catching a spawn that dies immediately.
///
/// `start` deliberately does NOT wait for readiness. `mlx_vlm.server` does not
/// begin serving HTTP until a preloaded `--model` has finished loading, and it
/// re-checks the Hub revision even for a cached checkpoint, so a 27B can take
/// minutes during which nothing answers. Blocking the RPC on that would hang
/// the Start button for the whole load.
///
/// So the pool records the server as `starting` and returns; the status poll
/// promotes it to `ready` when it answers, or to `crashed` if the process
/// exits. This window only catches an argv or binary error that kills the
/// process at once, which is worth reporting inline rather than as a state the
/// caller has to poll for.
const SPAWN_GRACE: Duration = Duration::from_millis(600);

/// Log lines returned per status request.
const LOG_TAIL: usize = 40;

struct RunningServer {
    process: MlxProcess,
    state: MlxServerState,
    /// Set once the server has answered, so a later silence reads as
    /// `Degraded` rather than as a fresh startup.
    has_been_ready: bool,
    detail: Option<String>,
    estimated_gib: Option<f64>,
}

/// One server's state, as the RPC surface and the UI see it.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct MlxServerStatus {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) port: Option<u16>,
    pub(crate) base_url: Option<String>,
    pub(crate) pid: Option<u32>,
    /// Checkpoint the server reports as resident, which is not the same as the
    /// configured `model`: with `model_discovery = "hf-cache"` a block can
    /// start with nothing loaded and serve on demand.
    pub(crate) loaded_model: Option<String>,
    /// The checkpoint the block is configured to launch with, which is not the
    /// same as `loaded_model`: under `model_discovery = "hf-cache"` a server
    /// can be configured for one checkpoint and holding none, or holding one a
    /// request asked for on the fly.
    pub(crate) configured_model: Option<String>,
    pub(crate) resident_gib: Option<f64>,
    pub(crate) estimated_gib: Option<f64>,
    pub(crate) models: Vec<String>,
    pub(crate) detail: Option<String>,
    /// Argv with the bearer token redacted, so "what did you actually run?"
    /// is answerable from the UI.
    pub(crate) command: Vec<String>,
}

impl MlxServerStatus {
    /// A block that is configured but not running.
    fn stopped(server: &MlxServerConfig) -> Self {
        Self {
            id: server.id.clone(),
            kind: if server.is_vlm() { "vlm" } else { "lm" }.to_string(),
            state: MlxServerState::Stopped.as_str().to_string(),
            port: (server.port != 0).then_some(server.port),
            base_url: None,
            pid: None,
            loaded_model: None,
            configured_model: non_empty(&server.model),
            resident_gib: None,
            estimated_gib: None,
            models: Vec::new(),
            detail: None,
            command: Vec::new(),
        }
    }
}

/// Supervises every managed MLX process.
#[derive(Default)]
pub(crate) struct MlxPool {
    running: Mutex<HashMap<String, RunningServer>>,
}

impl MlxPool {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Base URL of a running block, or `None` when it is not running.
    ///
    /// This is what closes the `port = 0` gap: only the pool knows the port a
    /// block was actually assigned.
    pub(crate) async fn base_url_for(&self, id: &str) -> Option<String> {
        let running = self.running.lock().await;
        let entry = running.get(id)?;
        Some(server_base_url(&entry.process))
    }

    /// Base URL for a block, preferring the live process and falling back to
    /// a fixed configured port.
    ///
    /// The fallback matters whenever the pool is empty but a server is up:
    /// after a core restart, and on every CLI invocation, which is a fresh
    /// process. A block with `port = 0` has no answer here, because only the
    /// process that spawned it ever knew the port it was assigned.
    pub(crate) async fn resolved_base_url(&self, config: &Config, id: &str) -> Option<String> {
        if let Some(live) = self.base_url_for(id).await {
            return Some(live);
        }
        let server = config.mlx.server(id)?;
        (server.port != 0).then(|| server.base_url(server.port))
    }

    /// Base URL of any ready block, preferring one that serves chat.
    ///
    /// Used by endpoint resolution so inference reaches a supervised server
    /// without the user having to pin a fixed port.
    pub(crate) async fn any_ready_base_url(&self) -> Option<String> {
        let running = self.running.lock().await;
        running
            .values()
            .find(|entry| entry.state.is_usable())
            .map(|entry| server_base_url(&entry.process))
    }

    /// Whether a block is running, and on which port.
    ///
    /// Lets a caller persist an auto-assigned port back into config, so the
    /// address survives a core restart instead of being knowable only to the
    /// process that spawned it.
    pub(crate) async fn assigned_port(&self, id: &str) -> Option<u16> {
        let running = self.running.lock().await;
        running.get(id).map(|entry| entry.process.port)
    }

    /// Start the block with this id.
    ///
    /// Refuses rather than spawning when the model would not fit alongside
    /// what is already resident.
    pub(crate) async fn start(
        &self,
        config: &Config,
        http: &reqwest::Client,
        id: &str,
    ) -> Result<MlxServerStatus, String> {
        let server = config
            .mlx
            .server(id)
            .ok_or_else(|| format!("no [[mlx.server]] block with id `{id}`"))?
            .clone();

        {
            let running = self.running.lock().await;
            if running.contains_key(id) {
                return Err(format!("server `{id}` is already running"));
            }
        }

        let estimated_gib = match admit(config, &server.model, &self.running_pids().await) {
            Admission::Allow { estimated_gib } => Some(estimated_gib),
            Admission::Unknown => None,
            Admission::Refuse { message } => return Err(message),
        };

        let mut process = spawn(config, &server).await?;

        log::info!(
            "[mlx] server `{id}` spawned pid={} port={}",
            process.pid,
            process.port
        );

        // Only long enough to notice a process that refuses to start at all,
        // e.g. an argv the binary rejects. Anything longer would be waiting on
        // the model load.
        tokio::time::sleep(SPAWN_GRACE).await;

        if let Some(exit) = process.exit_status() {
            // Its own stderr says far more than "the process exited" does.
            let tail = process.logs_tail(LOG_TAIL);
            let reason = tail
                .iter()
                .rev()
                .take(3)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ");
            clear_marker_for(config, &id);
            return Err(if reason.is_empty() {
                format!("server `{id}` {exit} immediately after starting")
            } else {
                format!("server `{id}` {exit} immediately after starting: {reason}")
            });
        }

        let (state, has_been_ready, detail) = (MlxServerState::Starting, false, None);

        self.running.lock().await.insert(
            id.to_string(),
            RunningServer {
                process,
                state,
                has_been_ready,
                detail,
                estimated_gib,
            },
        );

        self.status_of(config, http, id)
            .await
            .ok_or_else(|| format!("server `{id}` vanished immediately after starting"))
    }

    /// Stop the block with this id. Stopping something already stopped is fine.
    ///
    /// Falls back to the spawn marker when this process has no in-memory
    /// record. That is not a rare path: the CLI is a fresh process per
    /// invocation, so `mlx stop` never shares a pool with the `mlx start` that
    /// spawned the server. Without the fallback it reported success while
    /// leaving the server running and its weights resident, which is a worse
    /// outcome than an honest failure.
    pub(crate) async fn stop(&self, config: &Config, id: &str) -> bool {
        let entry = self.running.lock().await.remove(id);
        if let Some(mut entry) = entry {
            entry.process.stop(config).await;
            return true;
        }

        log::debug!("[mlx] stop `{id}`: no in-memory handle, checking the spawn marker");
        super::process::reclaim_orphan_if_ours(config, id)
    }

    /// Stop then start, picking up any config edits.
    pub(crate) async fn restart(
        &self,
        config: &Config,
        http: &reqwest::Client,
        id: &str,
    ) -> Result<MlxServerStatus, String> {
        // A restart does not care whether anything was running.
        let _ = self.stop(config, id).await;
        self.start(config, http, id).await
    }

    /// Stop every managed process. Called on shutdown.
    pub(crate) async fn shutdown_all(&self, config: &Config) {
        let mut running = self.running.lock().await;
        for (id, mut entry) in running.drain() {
            log::info!("[mlx] shutting down server `{id}`");
            entry.process.stop(config).await;
        }
    }

    /// Start every block marked `autostart` that is not already running.
    ///
    /// Failures are logged, not propagated: one misconfigured block must not
    /// stop the others from coming up.
    pub(crate) async fn reconcile(&self, config: &Config, http: &reqwest::Client) {
        if !config.mlx.enabled {
            log::debug!("[mlx] reconcile skipped: mlx.enabled is false");
            return;
        }

        for server in &config.mlx.servers {
            if !server.autostart {
                continue;
            }
            if self.running.lock().await.contains_key(&server.id) {
                continue;
            }
            if let Err(err) = self.start(config, http, &server.id).await {
                log::warn!("[mlx] autostart of `{}` failed: {err}", server.id);
            }
        }
    }

    /// Status of every configured block, running or not.
    pub(crate) async fn status_all(
        &self,
        config: &Config,
        http: &reqwest::Client,
    ) -> Vec<MlxServerStatus> {
        let mut out = Vec::with_capacity(config.mlx.servers.len());
        for server in &config.mlx.servers {
            match self.status_of(config, http, &server.id).await {
                Some(status) => out.push(status),
                None => out.push(MlxServerStatus::stopped(server)),
            }
        }
        out
    }

    /// Status of one running block, re-probing it. `None` when not running.
    pub(crate) async fn status_of(
        &self,
        config: &Config,
        http: &reqwest::Client,
        id: &str,
    ) -> Option<MlxServerStatus> {
        let server_config = config.mlx.server(id)?.clone();
        let bearer = server_config
            .uses_bearer()
            .then(|| server_config.api_key.trim().to_string());

        // Snapshot what the probe needs, then release the lock: probing takes
        // seconds, and holding the map would block every other caller.
        let (base_url, pid, command, estimated_gib, has_been_ready) = {
            let running = self.running.lock().await;
            let entry = running.get(id)?;
            (
                server_base_url(&entry.process),
                entry.process.pid,
                entry.process.redacted_argv.clone(),
                entry.estimated_gib,
                entry.has_been_ready,
            )
        };

        let report = probe_models(http, &base_url, bearer.as_deref()).await;
        let liveness = if server_config.is_vlm() {
            probe_liveness(http, &base_url).await
        } else {
            None
        };

        let mut running = self.running.lock().await;
        let entry = running.get_mut(id)?;

        let process_alive = entry.process.exit_status().is_none();
        let state = classify(&report, process_alive, has_been_ready);

        entry.state = state;
        entry.has_been_ready = has_been_ready || state == MlxServerState::Ready;
        entry.detail = if state == MlxServerState::Crashed {
            // The stderr tail is the only useful thing about a crash.
            let tail = entry.process.logs_tail(LOG_TAIL);
            Some(
                tail.iter()
                    .rev()
                    .take(3)
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" | "),
            )
        } else {
            report.detail.clone()
        };

        Some(MlxServerStatus {
            id: id.to_string(),
            kind: if server_config.is_vlm() { "vlm" } else { "lm" }.to_string(),
            state: state.as_str().to_string(),
            port: Some(entry.process.port),
            base_url: Some(base_url),
            pid: Some(pid),
            loaded_model: liveness.as_ref().and_then(|l| l.loaded_model.clone()),
            configured_model: non_empty(&server_config.model),
            resident_gib: Some(resident_gib(&[pid])),
            estimated_gib,
            models: report.models,
            detail: entry.detail.clone(),
            command,
        })
    }

    /// Recent output from one block.
    pub(crate) async fn logs(&self, id: &str, limit: usize) -> Vec<String> {
        let running = self.running.lock().await;
        running
            .get(id)
            .map(|entry| entry.process.logs_tail(limit))
            .unwrap_or_default()
    }

    /// Memory in use by managed servers, and the ceiling they share.
    pub(crate) async fn memory_summary(&self, config: &Config) -> (f64, f64) {
        (resident_gib(&self.running_pids().await), budget_gib(config))
    }

    async fn running_pids(&self) -> Vec<u32> {
        self.running
            .lock()
            .await
            .values()
            .map(|entry| entry.process.pid)
            .collect()
    }
}

/// Drop the spawn marker for a server that failed to start, so a later stop
/// does not try to reclaim a PID that is already gone.
fn clear_marker_for(config: &Config, id: &str) {
    super::process::reclaim_orphan_if_ours(config, id);
}

/// `None` for a blank slot, so the UI can tell "not configured" from "".
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Base URL for a running process, always loopback-rooted and `/v1`-suffixed.
fn server_base_url(process: &MlxProcess) -> String {
    format!("http://127.0.0.1:{}/v1", process.port)
}
