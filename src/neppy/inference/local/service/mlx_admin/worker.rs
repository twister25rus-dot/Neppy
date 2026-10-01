//! Worker control: lazy start, crash restart budget, model switch, unload.
//!
//! The controller is the long-lived core process; the MLX server is the
//! worker, and it only needs to run while there is work. `ensure_started` is
//! what a gated request calls before it reaches the server: it reaps a crashed
//! process, respawns within a restart budget, admits the load against
//! measured available memory (through `MlxPool::start` → `memory::admit`),
//! and waits for readiness. `prepare_model` then makes sure at most one chat
//! model is resident by unloading a different one first.
//!
//! Nothing here runs unless `mlx.enabled` is set: a server the user started by
//! hand is theirs, and the gate still serializes requests to it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;

use crate::neppy::config::Config;

use super::super::LocalAiService;
use super::health::{probe_liveness, probe_models};
use super::memory::AdmitMode;
use super::metrics::{event, MetricsSink, OllamaLoaded};
use super::pressure::{gpu_in_use_bytes, sample_system, PressureTracker};

/// Window over which crash restarts are counted.
const RESTART_WINDOW: Duration = Duration::from_secs(600);
/// Readiness poll interval while a worker starts.
const READY_POLL: Duration = Duration::from_millis(1000);
/// Minimum spacing of `ioreg` GPU samples.
const GPU_SAMPLE_EVERY: Duration = Duration::from_secs(10);
/// Minimum spacing of Ollama `/api/ps` probes.
const OLLAMA_PROBE_EVERY: Duration = Duration::from_secs(10);

/// Per-service worker state that is not the gate or the metrics sink.
pub(crate) struct WorkerControl {
    pub(crate) tracker: Mutex<PressureTracker>,
    restarts: Mutex<VecDeque<Instant>>,
    /// Set when a crash was reaped; the next spawn counts as a restart.
    crash_pending: AtomicBool,
    /// Serializes `ensure_started` so two waiters cannot both spawn.
    ensure_lock: tokio::sync::Mutex<()>,
    gpu: Mutex<Option<(Instant, Option<u64>)>>,
    ollama: Mutex<Option<(Instant, Vec<OllamaLoaded>)>>,
    /// Whether to probe and (under pressure) unload the local Ollama daemon.
    /// Off in unit tests, which must never reach a real daemon.
    observe_ollama: bool,
}

impl Default for WorkerControl {
    fn default() -> Self {
        Self {
            tracker: Mutex::new(PressureTracker::new()),
            restarts: Mutex::new(VecDeque::new()),
            crash_pending: AtomicBool::new(false),
            ensure_lock: tokio::sync::Mutex::new(()),
            gpu: Mutex::new(None),
            ollama: Mutex::new(None),
            observe_ollama: !cfg!(test),
        }
    }
}

impl WorkerControl {
    pub(crate) fn note_crash(&self) {
        self.crash_pending.store(true, Ordering::SeqCst);
    }

    /// Record a restart at `now` if the budget allows one; `false` when
    /// `max` restarts already happened inside the window.
    pub(crate) fn try_restart(&self, now: Instant, max: u32) -> bool {
        let mut restarts = self.restarts.lock();
        while restarts
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= RESTART_WINDOW)
        {
            restarts.pop_front();
        }
        if restarts.len() >= max as usize {
            return false;
        }
        restarts.push_back(now);
        true
    }

    pub(crate) fn restarts_in_window(&self) -> usize {
        let now = Instant::now();
        self.restarts
            .lock()
            .iter()
            .filter(|at| now.saturating_duration_since(**at) < RESTART_WINDOW)
            .count()
    }

    /// GPU in-use bytes, re-sampled at most every ten seconds.
    pub(crate) async fn gpu_in_use(&self) -> Option<u64> {
        if let Some((at, value)) = *self.gpu.lock() {
            if at.elapsed() < GPU_SAMPLE_EVERY {
                return value;
            }
        }
        let value = tokio::task::spawn_blocking(gpu_in_use_bytes)
            .await
            .ok()
            .flatten();
        *self.gpu.lock() = Some((Instant::now(), value));
        value
    }

    /// Models Ollama holds resident, probed at most every ten seconds.
    pub(crate) async fn ollama_loaded(&self, http: &reqwest::Client) -> Vec<OllamaLoaded> {
        if !self.observe_ollama {
            return Vec::new();
        }
        if let Some((at, models)) = self.ollama.lock().as_ref() {
            if at.elapsed() < OLLAMA_PROBE_EVERY {
                return models.clone();
            }
        }
        let models = probe_ollama(http).await;
        *self.ollama.lock() = Some((Instant::now(), models.clone()));
        models
    }
}

/// The `[[mlx.server]]` block the worker policy manages: `primary` when it
/// exists, otherwise the first block.
pub(crate) fn worker_server_id(config: &Config) -> Option<String> {
    config
        .mlx
        .server("primary")
        .or_else(|| config.mlx.servers.first())
        .map(|server| server.id.clone())
}

/// Point the metrics sink at the current workspace and bounds.
pub(crate) fn configure_metrics(metrics: &MetricsSink, config: &Config) {
    let worker = &config.mlx.worker;
    metrics.configure(
        MetricsSink::dir_for_workspace(&config.workspace_dir),
        worker.metrics_file_cap_mib,
        worker.metrics_retention_days,
    );
}

/// Make sure the managed worker is running and answering.
///
/// A no-op when the supervisor is off. Reaps a crashed process first; a
/// respawn after a crash counts against `max_restarts_per_10min`.
pub(crate) async fn ensure_started(
    svc: &Arc<LocalAiService>,
    config: &Config,
) -> Result<(), String> {
    if !config.mlx.enabled {
        return Ok(());
    }
    super::watchdog::spawn_watchdog(Arc::clone(svc));
    configure_metrics(&svc.metrics, config);

    let id = worker_server_id(config).ok_or("no [[mlx.server]] block is configured")?;
    let _serial = svc.worker.ensure_lock.lock().await;
    let timeout = Duration::from_secs(config.mlx.worker.first_token_timeout_secs.max(1));

    match svc.mlx.process_of(&id).await {
        Some(process) if process.alive => {
            log::debug!("[mlx:worker] ensure `{id}`: running pid={}", process.pid);
            return wait_ready(svc, config, &id, timeout).await;
        }
        Some(process) => {
            log::warn!("[mlx:worker] ensure `{id}`: pid={} has exited", process.pid);
            svc.mlx.reap_crashed(config).await;
            svc.worker.note_crash();
        }
        None => {
            // Not ours. A server another process started (an earlier core, or
            // the user by hand) is used as it is, never replaced.
            if let Some(base_url) = svc.mlx.resolved_base_url(config, &id).await {
                let bearer = config
                    .mlx
                    .server(&id)
                    .and_then(|s| s.uses_bearer().then(|| s.api_key.trim().to_string()));
                if probe_models(&svc.http, &base_url, bearer.as_deref())
                    .await
                    .reachable
                {
                    log::debug!("[mlx:worker] ensure `{id}`: reachable, not supervised here");
                    return Ok(());
                }
            }
        }
    }

    if svc.worker.crash_pending.load(Ordering::SeqCst) {
        let max = config.mlx.worker.max_restarts_per_10min;
        if !svc.worker.try_restart(Instant::now(), max) {
            let message = format!(
                "MLX worker `{id}` crashed {max} times in ten minutes; not restarting it until \
                 the window passes. Check `mlx.logs`."
            );
            log::warn!("[mlx:worker] {message}");
            return Err(message);
        }
        svc.metrics
            .event(event::WORKER_RESTART, Some(&id), "respawn after crash");
    }

    log::info!("[mlx:worker] lazily starting `{id}`");
    svc.mlx
        .start_with(config, &svc.http, &id, AdmitMode::Managed)
        .await
        .map(|_| ())?;
    svc.worker.crash_pending.store(false, Ordering::SeqCst);
    svc.worker
        .tracker
        .lock()
        .set_swap_baseline(Some(sample_system().swap_used_bytes));
    svc.gate.touch();
    wait_ready(svc, config, &id, timeout).await
}

/// Poll the pool's status until `id` answers, crashes, or `timeout` passes.
async fn wait_ready(
    svc: &Arc<LocalAiService>,
    config: &Config,
    id: &str,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    let mut waited = false;
    loop {
        let status = svc.mlx.status_of(config, &svc.http, id).await;
        match status.as_ref().map(|s| s.state.as_str()) {
            Some("ready") => {
                if waited {
                    svc.metrics.event(
                        event::MODEL_READY,
                        Some(id),
                        format!("after {}ms", started.elapsed().as_millis()),
                    );
                }
                return Ok(());
            }
            Some("crashed") => {
                svc.worker.note_crash();
                let detail = status.and_then(|s| s.detail).unwrap_or_default();
                return Err(format!(
                    "MLX worker `{id}` crashed while starting: {detail}"
                ));
            }
            None => return Err(format!("MLX worker `{id}` is not running")),
            _ => {}
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "MLX worker `{id}` did not become ready within {}s",
                timeout.as_secs()
            ));
        }
        waited = true;
        tokio::time::sleep(READY_POLL).await;
    }
}

/// Whether two model references name the same checkpoint. The server reports
/// what it was given, which may be a repo id or a local snapshot path.
pub(crate) fn same_model(loaded: &str, wanted: &str) -> bool {
    let (loaded, wanted) = (loaded.trim(), wanted.trim());
    if loaded.eq_ignore_ascii_case(wanted) {
        return true;
    }
    let cache_dir = format!("models--{}", wanted.replace('/', "--"));
    let lower = loaded.to_ascii_lowercase();
    lower.contains(&cache_dir.to_ascii_lowercase())
        || lower.ends_with(&format!("/{}", wanted.to_ascii_lowercase()))
}

/// Keep at most one chat model resident: unload a different loaded model
/// before a request for `model_id`. Returns whether the request will load a
/// model (so the caller can record `model_ready` afterwards).
pub(crate) async fn prepare_model(
    svc: &Arc<LocalAiService>,
    config: &Config,
    model_id: &str,
) -> Result<bool, String> {
    let Some(id) = worker_server_id(config) else {
        return Ok(false);
    };
    if !config.mlx.server(&id).is_some_and(|s| s.is_vlm()) {
        return Ok(false); // mlx_lm.server has no /health or /unload.
    }
    let Some(base_url) = svc.mlx.resolved_base_url(config, &id).await else {
        return Ok(false);
    };
    let Some(live) = probe_liveness(&svc.http, &base_url).await else {
        return Ok(false);
    };
    match live.loaded_model.as_deref() {
        Some(loaded) if same_model(loaded, model_id) => Ok(false),
        Some(loaded) => {
            // Only a worker this process holds may be unloaded. One it does not
            // (the user's own server, another core's) is used as it stands, the
            // same rule `ensure_started` applies; unloading it would take the
            // model out from under whoever started it.
            if !svc.mlx.is_running(&id).await {
                log::warn!(
                    "[mlx:worker] `{id}` has `{loaded}` loaded and is not supervised here; refusing to unload it for {model_id}"
                );
                return Err(format!(
                    "the MLX server `{id}` was not started by Neppy and has `{loaded}` loaded; \
                     Neppy will not unload it to load {model_id}. Stop that server, or use the \
                     model it has loaded."
                ));
            }
            log::info!("[mlx:worker] model switch on `{id}`: unloading before {model_id}");
            unload(svc, &base_url).await?;
            svc.metrics.event(
                event::MODEL_UNLOAD,
                Some(&id),
                format!("switch from {loaded} to {model_id}"),
            );
            svc.metrics.event(
                event::MODEL_LOAD_START,
                Some(&id),
                format!("model={model_id}"),
            );
            Ok(true)
        }
        None => {
            svc.metrics.event(
                event::MODEL_LOAD_START,
                Some(&id),
                format!("model={model_id} (lazy)"),
            );
            Ok(true)
        }
    }
}

/// `POST /unload` on the server at `base_url` (which ends in `/v1`).
pub(crate) async fn unload(svc: &LocalAiService, base_url: &str) -> Result<(), String> {
    let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
    let response = svc
        .http
        .post(format!("{root}/unload"))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|err| format!("unload request failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "server refused the unload: HTTP {}",
            response.status()
        ));
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct OllamaPs {
    #[serde(default)]
    models: Vec<OllamaPsModel>,
}

#[derive(Debug, Default, Deserialize)]
struct OllamaPsModel {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
}

/// Parse an Ollama `/api/ps` body.
pub(crate) fn parse_ollama_ps(body: &str) -> Vec<OllamaLoaded> {
    serde_json::from_str::<OllamaPs>(body)
        .map(|ps| {
            ps.models
                .into_iter()
                .filter(|m| !m.name.is_empty())
                .map(|m| OllamaLoaded {
                    name: m.name,
                    size_mib: m.size / (1024 * 1024),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn ollama_root() -> String {
    let base = crate::neppy::inference::local::ollama_base_url();
    base.trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string()
}

async fn probe_ollama(http: &reqwest::Client) -> Vec<OllamaLoaded> {
    let url = format!("{}/api/ps", ollama_root());
    let Ok(response) = http.get(&url).timeout(Duration::from_secs(2)).send().await else {
        return Vec::new();
    };
    match response.text().await {
        Ok(body) => parse_ollama_ps(&body),
        Err(_) => Vec::new(),
    }
}

/// Ask Ollama to drop every resident model (`keep_alive: 0`). Ollama is
/// observed, not managed; this is the one thing done to it, under pressure.
pub(crate) async fn unload_ollama(svc: &LocalAiService, models: &[OllamaLoaded]) {
    let url = format!("{}/api/generate", ollama_root());
    for model in models {
        let body = serde_json::json!({ "model": model.name, "keep_alive": 0 });
        let result = svc
            .http
            .post(&url)
            .timeout(Duration::from_secs(10))
            .json(&body)
            .send()
            .await;
        let detail = match result {
            Ok(r) if r.status().is_success() => format!("{} ({} MiB)", model.name, model.size_mib),
            Ok(r) => format!("{}: HTTP {}", model.name, r.status()),
            Err(err) => format!("{}: {err}", model.name),
        };
        svc.metrics.event(event::OLLAMA_UNLOAD, None, detail);
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
