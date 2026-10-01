//! The idle/pressure watchdog.
//!
//! One background task per process samples memory, folds it into the
//! pressure state, pauses or preempts the gate, and unloads, stops or reaps
//! the worker. The policy is [`decide`], which is pure and table-tested; the
//! loop only gathers inputs and applies the verdict.
//!
//! Cadence follows what can change: every 2 s while a request is active,
//! 10 s while the worker is up or the gate is paused, 30 s while stopped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::neppy::config::Config;

use super::super::LocalAiService;
use super::health::{probe_in_flight, probe_liveness, server_busy};
use super::memory::budget_gib;
use super::metrics::{event, MetricsSample};
use super::models::BYTES_PER_GIB;
use super::pressure::{sample_process, sample_system, PressureState, ProcMem, SystemMemory};
use super::worker::{configure_metrics, unload, unload_ollama, worker_server_id};

const MIB: u64 = 1024 * 1024;

/// What the worker process is doing, as far as the watchdog can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerPhase {
    /// Not held by this process.
    Stopped,
    /// Alive, not answering `/health` yet.
    Starting,
    /// Alive with a model resident.
    Loaded,
    /// Alive with nothing resident (after `/unload`).
    Unloaded,
    /// Held, but the process has exited.
    Crashed,
}

impl WorkerPhase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Loaded => "loaded",
            Self::Unloaded => "unloaded",
            Self::Crashed => "crashed",
        }
    }

    fn is_alive(self) -> bool {
        matches!(self, Self::Starting | Self::Loaded | Self::Unloaded)
    }
}

/// The verdict of one watchdog tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkerAction {
    None,
    /// Free the weights, keep the process (`POST /unload`).
    Unload,
    /// Stop the process once no request holds the gate.
    Stop,
    /// Stop the process now; the in-flight request has been preempted.
    StopNow,
    /// Remove a crashed process from the pool.
    ReapCrashed,
    /// Lift a pressure pause on the gate.
    Resume,
}

/// Everything [`decide`] looks at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WatchInputs {
    pub(crate) pressure: PressureState,
    pub(crate) idle_for: Duration,
    pub(crate) worker: WorkerPhase,
    pub(crate) gate_active: bool,
    pub(crate) gate_paused: bool,
    /// The server reports queued or in-flight work from any client, gated or
    /// not (vision, STT and TTS reach it directly). Treated as activity: no
    /// Unload or Stop while it holds. Critical still stops.
    pub(crate) server_busy: bool,
    pub(crate) pressure_only: bool,
    pub(crate) idle_unload_secs: u64,
    pub(crate) idle_stop_secs: u64,
}

/// The worker policy. Pure: same inputs, same action.
pub(crate) fn decide(inputs: &WatchInputs) -> WorkerAction {
    let alive = inputs.worker.is_alive();
    if inputs.worker == WorkerPhase::Crashed {
        return WorkerAction::ReapCrashed;
    }
    match inputs.pressure {
        PressureState::Critical if alive => return WorkerAction::StopNow,
        // Let the in-flight step finish; the gate is paused, so it is the last.
        PressureState::Elevated if alive && (inputs.gate_active || inputs.server_busy) => {
            return WorkerAction::None
        }
        PressureState::Elevated if alive => return WorkerAction::Stop,
        PressureState::Critical | PressureState::Elevated => return WorkerAction::None,
        PressureState::Normal => {}
    }
    if inputs.gate_paused {
        return WorkerAction::Resume;
    }
    if !alive || inputs.gate_active || inputs.server_busy || inputs.pressure_only {
        return WorkerAction::None;
    }
    let idle = inputs.idle_for;
    if inputs.idle_stop_secs > 0 && idle >= Duration::from_secs(inputs.idle_stop_secs) {
        return WorkerAction::Stop;
    }
    if inputs.worker == WorkerPhase::Loaded
        && inputs.idle_unload_secs > 0
        && idle >= Duration::from_secs(inputs.idle_unload_secs)
    {
        return WorkerAction::Unload;
    }
    WorkerAction::None
}

static STARTED: AtomicBool = AtomicBool::new(false);

/// Start the watchdog for `svc`. Idempotent; a no-op outside a tokio runtime.
pub(crate) fn spawn_watchdog(svc: Arc<LocalAiService>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        log::debug!("[mlx:worker] watchdog not started: no tokio runtime");
        return;
    };
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    log::info!("[mlx:worker] watchdog started");
    handle.spawn(async move {
        loop {
            let pause = match crate::neppy::config::rpc::load_config_with_timeout().await {
                Ok(config) => tick(&svc, &config).await,
                Err(err) => {
                    log::debug!("[mlx:worker] watchdog could not load config: {err}");
                    Duration::from_secs(30)
                }
            };
            tokio::time::sleep(pause).await;
        }
    });
}

/// Start the watchdog from anywhere in the core, loading config itself.
pub fn spawn_worker_watchdog() {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        log::debug!("[mlx:worker] spawn_worker_watchdog called outside a runtime");
        return;
    };
    handle.spawn(async {
        match crate::neppy::config::rpc::load_config_with_timeout().await {
            Ok(config) => {
                spawn_watchdog(crate::neppy::inference::local::global(&config));
            }
            Err(err) => log::warn!("[mlx:worker] watchdog not started: {err}"),
        }
    });
}

/// One watchdog pass. Returns how long to sleep before the next.
pub(crate) async fn tick(svc: &Arc<LocalAiService>, config: &Config) -> Duration {
    if !config.mlx.enabled {
        return Duration::from_secs(30);
    }
    let Some(id) = worker_server_id(config) else {
        return Duration::from_secs(30);
    };
    let cfg = &config.mlx.worker;
    configure_metrics(&svc.metrics, config);

    let system = sample_system();
    let process = svc.mlx.process_of(&id).await;
    let worker_mem = process
        .filter(|p| p.alive)
        .and_then(|p| sample_process(p.pid));
    let budget_bytes = (budget_gib(config) * BYTES_PER_GIB) as u64;
    let (pressure, transition) = svc.worker.tracker.lock().observe(
        &system,
        worker_mem.map(|m| m.footprint_bytes),
        budget_bytes,
        Instant::now(),
        cfg,
    );

    if let Some(transition) = transition {
        log::info!(
            "[mlx:worker] pressure {} -> {}: {}",
            transition.from.as_str(),
            transition.to.as_str(),
            transition.reason
        );
        svc.metrics.event(
            event::PRESSURE_TRANSITION,
            Some(&id),
            format!(
                "{} -> {}: {}",
                transition.from.as_str(),
                transition.to.as_str(),
                transition.reason
            ),
        );
        if transition.to > PressureState::Normal {
            svc.gate.pause(transition.to);
            if transition.to == PressureState::Critical && svc.gate.snapshot().active > 0 {
                svc.gate.preempt();
                svc.metrics
                    .event(event::REQUEST_PREEMPTED, Some(&id), "memory critical");
            }
            if transition.from == PressureState::Normal {
                let models = svc.worker.ollama_loaded(&svc.http).await;
                unload_ollama(svc, &models).await;
            }
        }
    }

    let alive = process.is_some_and(|p| p.alive);
    let is_vlm = config.mlx.server(&id).is_some_and(|s| s.is_vlm());
    let liveness = match (alive, svc.mlx.resolved_base_url(config, &id).await) {
        (true, Some(base)) if is_vlm => probe_liveness(&svc.http, &base).await,
        _ => None,
    };
    let phase = match process {
        None => WorkerPhase::Stopped,
        Some(p) if !p.alive => WorkerPhase::Crashed,
        Some(_) => match &liveness {
            Some(live) if live.has_resident_model() => WorkerPhase::Loaded,
            Some(_) => WorkerPhase::Unloaded,
            // mlx_lm.server has no /health; assume what it serves is loaded.
            None if !is_vlm => WorkerPhase::Loaded,
            None => WorkerPhase::Starting,
        },
    };

    let gate = svc.gate.snapshot();
    let mut inputs = WatchInputs {
        pressure,
        idle_for: gate.idle_for,
        worker: phase,
        gate_active: gate.active > 0,
        gate_paused: gate.paused.is_some(),
        server_busy: false,
        pressure_only: cfg.pressure_only(),
        idle_unload_secs: cfg.idle_unload_secs,
        idle_stop_secs: cfg.idle_stop_secs,
    };
    let mut action = decide(&inputs);
    // Only a verdict that would take the model away is worth a probe.
    if matches!(action, WorkerAction::Unload | WorkerAction::Stop) {
        let queue_depth = liveness.as_ref().and_then(|l| l.request_queue_depth);
        let in_flight = match svc.mlx.resolved_base_url(config, &id).await {
            Some(base) => {
                let bearer = config
                    .mlx
                    .server(&id)
                    .and_then(|s| s.uses_bearer().then(|| s.api_key.trim().to_string()));
                probe_in_flight(&svc.http, &base, bearer.as_deref()).await
            }
            None => None,
        };
        inputs.server_busy = server_busy(queue_depth, in_flight);
        if inputs.server_busy {
            action = decide(&inputs);
            log::debug!(
                "[mlx:worker] deferred unload: server busy (queue={queue_depth:?} in_flight={in_flight:?}) -> {action:?}"
            );
            // Work the gate never saw is still work: restart the idle clock.
            svc.gate.touch();
        }
    }
    if action != WorkerAction::None {
        log::debug!(
            "[mlx:worker] tick: pressure={} phase={} idle={}s active={} -> {action:?}",
            pressure.as_str(),
            phase.as_str(),
            gate.idle_for.as_secs(),
            gate.active
        );
    }
    apply(svc, config, &id, action, pressure).await;

    record_sample(
        svc,
        &system,
        process.map(|p| p.pid),
        worker_mem,
        phase,
        pressure,
        liveness,
    )
    .await;

    if gate.active > 0 {
        Duration::from_secs(2)
    } else if phase.is_alive() || gate.paused.is_some() {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(30)
    }
}

/// Footprints below this are not worth judging an unload by.
const UNLOAD_JUDGED_ABOVE_BYTES: u64 = 1024 * MIB;

/// Whether an `/unload` actually gave memory back: the footprint at least
/// halved. A worker that was small to begin with is never judged ineffective.
pub(crate) fn unload_released_memory(before: u64, after: u64) -> bool {
    before < UNLOAD_JUDGED_ABOVE_BYTES || after.saturating_mul(2) <= before
}

async fn worker_footprint(svc: &Arc<LocalAiService>, id: &str) -> Option<u64> {
    let process = svc.mlx.process_of(id).await.filter(|p| p.alive)?;
    sample_process(process.pid).map(|m| m.footprint_bytes)
}

async fn apply(
    svc: &Arc<LocalAiService>,
    config: &Config,
    id: &str,
    action: WorkerAction,
    pressure: PressureState,
) {
    match action {
        WorkerAction::None => {}
        WorkerAction::Resume => svc.gate.resume(),
        WorkerAction::Unload => {
            let Some(base) = svc.mlx.resolved_base_url(config, id).await else {
                return;
            };
            let before = worker_footprint(svc, id).await;
            match unload(svc, &base).await {
                Ok(()) => {
                    svc.metrics.event(event::MODEL_UNLOAD, Some(id), "idle");
                    // `/unload` clears the server's model registry and MLX's
                    // buffer cache, but measured on mlx_vlm 0.7.0 the 9 GiB of
                    // Metal weight buffers stay resident in the process. An
                    // unload that frees nothing is not an idle policy, so stop
                    // the process instead; the next request respawns it.
                    let after = worker_footprint(svc, id).await;
                    if let (Some(before), Some(after)) = (before, after) {
                        if !unload_released_memory(before, after) {
                            log::warn!(
                                "[mlx:worker] unload of `{id}` left {} MiB resident (was {} MiB); stopping the process to return the memory",
                                after / MIB,
                                before / MIB
                            );
                            svc.metrics.event(
                                event::UNLOAD_INEFFECTIVE,
                                Some(id),
                                format!(
                                    "footprint {} -> {} MiB; stopping the worker instead",
                                    before / MIB,
                                    after / MIB
                                ),
                            );
                            svc.mlx.stop(config, id).await;
                            svc.worker.tracker.lock().set_swap_baseline(None);
                        }
                    }
                }
                Err(err) => log::warn!("[mlx:worker] idle unload of `{id}` failed: {err}"),
            }
        }
        WorkerAction::Stop | WorkerAction::StopNow => {
            log::info!(
                "[mlx:worker] stopping `{id}` ({action:?}, pressure {})",
                pressure.as_str()
            );
            svc.mlx.stop(config, id).await;
            svc.worker.tracker.lock().set_swap_baseline(None);
        }
        WorkerAction::ReapCrashed => {
            let reaped = svc.mlx.reap_crashed(config).await;
            if !reaped.is_empty() {
                svc.worker.note_crash();
            }
        }
    }
}

async fn record_sample(
    svc: &Arc<LocalAiService>,
    system: &SystemMemory,
    pid: Option<u32>,
    worker_mem: Option<ProcMem>,
    phase: WorkerPhase,
    pressure: PressureState,
    liveness: Option<super::health::LivenessReport>,
) {
    let gate = svc.gate.snapshot();
    let usage = svc.metrics.last_usage();
    let core = sample_process(std::process::id());
    let live = liveness.unwrap_or_default();
    let sample = MetricsSample {
        ts_ms: chrono::Utc::now().timestamp_millis(),
        avail_pct: system.avail_pct,
        pressure_level: system.pressure_level,
        pressure_state: pressure.as_str().to_string(),
        swap_used_mib: system.swap_used_bytes / MIB,
        compressed_mib: system.compressed_bytes / MIB,
        worker_pid: pid,
        worker_footprint_mib: worker_mem.map(|m| m.footprint_bytes / MIB),
        worker_rss_mib: worker_mem.map(|m| m.rss_bytes / MIB),
        worker_state: phase.as_str().to_string(),
        loaded_model: live.loaded_model.clone(),
        ollama_loaded: svc.worker.ollama_loaded(&svc.http).await,
        gate_active: gate.active,
        gate_waiting: gate.waiting,
        server_queue_depth: live.request_queue_depth,
        context_limit: live.effective_context_limit,
        last_prompt_tokens: usage.prompt_tokens,
        last_completion_tokens: usage.completion_tokens,
        core_rss_mib: core.map(|m| m.rss_bytes / MIB),
        core_footprint_mib: core.map(|m| m.footprint_bytes / MIB),
        gpu_in_use_mib: svc.worker.gpu_in_use().await.map(|b| b / MIB),
    };
    svc.metrics.record_sample(sample);
}

#[cfg(test)]
#[path = "watchdog_tests.rs"]
mod tests;
