//! Payloads for `mlx.worker_status` and `mlx.worker_metrics`.
//!
//! Kept here rather than in `mlx_schemas.rs` so the RPC file stays thin: the
//! handlers there parse params and call these.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::neppy::config::schema::MlxWorkerConfig;
use crate::neppy::config::Config;

use super::super::LocalAiService;
use super::gate::GateSnapshot;
use super::metrics::{MetricsSample, MetricsWindow};
use super::pressure::{sample_process, sample_system, SystemMemory};
use super::worker::{configure_metrics, worker_server_id};

/// Upper bound on entries of each kind one `worker_metrics` call returns.
pub(crate) const MAX_METRICS_LIMIT: usize = 2000;
const DEFAULT_METRICS_LIMIT: usize = 200;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct WorkerMetricsParams {
    #[serde(default)]
    pub(crate) since_ms: Option<i64>,
    #[serde(default)]
    pub(crate) limit: Option<usize>,
    #[serde(default)]
    pub(crate) events_only: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkerProcessStatus {
    pub(crate) pid: u32,
    pub(crate) alive: bool,
    pub(crate) footprint_mib: Option<u64>,
    pub(crate) rss_mib: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WorkerStatus {
    pub(crate) supervised: bool,
    pub(crate) server_id: Option<String>,
    pub(crate) pressure_state: String,
    pub(crate) system: SystemMemory,
    pub(crate) worker: Option<WorkerProcessStatus>,
    pub(crate) gate: GateSnapshot,
    pub(crate) restarts_in_window: usize,
    pub(crate) last_sample: Option<MetricsSample>,
    pub(crate) policy: MlxWorkerConfig,
}

/// Current worker state. Also starts the watchdog when the supervisor is on,
/// so asking for status is enough to bring the controller up.
pub(crate) async fn worker_status(svc: &Arc<LocalAiService>, config: &Config) -> WorkerStatus {
    if config.mlx.enabled {
        super::watchdog::spawn_watchdog(Arc::clone(svc));
    }
    configure_metrics(&svc.metrics, config);
    let server_id = worker_server_id(config);
    let worker = match &server_id {
        Some(id) => svc.mlx.process_of(id).await.map(|p| {
            let mem = p.alive.then(|| sample_process(p.pid)).flatten();
            WorkerProcessStatus {
                pid: p.pid,
                alive: p.alive,
                footprint_mib: mem.map(|m| m.footprint_bytes / MIB),
                rss_mib: mem.map(|m| m.rss_bytes / MIB),
            }
        }),
        None => None,
    };
    log::debug!(
        "[mlx:worker] status requested server={server_id:?} running={}",
        worker.as_ref().is_some_and(|w| w.alive)
    );
    WorkerStatus {
        supervised: config.mlx.enabled,
        server_id,
        pressure_state: svc.worker.tracker.lock().state().as_str().to_string(),
        system: sample_system(),
        worker,
        gate: svc.gate.snapshot(),
        restarts_in_window: svc.worker.restarts_in_window(),
        last_sample: svc.metrics.latest_sample(),
        policy: config.mlx.worker.clone(),
    }
}

/// Recent samples and events from the in-memory rings.
pub(crate) fn worker_metrics(svc: &LocalAiService, params: &WorkerMetricsParams) -> MetricsWindow {
    let limit = params
        .limit
        .unwrap_or(DEFAULT_METRICS_LIMIT)
        .clamp(1, MAX_METRICS_LIMIT);
    svc.metrics.recent(
        params.since_ms.unwrap_or(0),
        limit,
        params.events_only.unwrap_or(false),
    )
}
