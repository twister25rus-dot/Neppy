//! The single-flight inference gate.
//!
//! Every `mlx:` model call — chat, the memory summariser, heartbeat, triage,
//! the assistant — takes the one permit here before it reaches the worker, so
//! at most one request is active on it. KV cache is per sequence, so a second
//! concurrent request is a second slice of memory the policy has not admitted.
//!
//! Waiting is FIFO (tokio's `Semaphore` is fair) and bounded: `max_waiters`
//! callers may queue, one more is refused as busy instead of piling up.
//! The permit is RAII. It is released on every path — success, error, a
//! stream dropped part-way, a preempted call — because releasing is `Drop`,
//! not a call someone has to remember.
//!
//! Ordering with the cron scheduler gate is always scheduler slot first, then
//! this gate, and nothing here ever waits on the scheduler, so the two cannot
//! deadlock.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::neppy::config::schema::MlxWorkerConfig;

use super::pressure::PressureState;

/// Stable prefix on model errors produced by the gate, so callers holding
/// only a `TinyAgentsError` can recover the [`GateError`].
pub(crate) const GATE_ERROR_PREFIX: &str = "[mlx:gate]";

/// Why a caller did not get (or lost) the inference slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GateError {
    /// `max_waiters` callers are already queued.
    Busy,
    /// Waited `acquire_timeout_secs` without getting the slot.
    Timeout,
    /// Waited out the timeout while memory pressure had paused the gate.
    Paused(PressureState),
    /// The request was cancelled because memory became critical.
    Preempted,
}

impl GateError {
    /// Message carried inside a model error. Round-trips through
    /// [`GateError::from_message`].
    pub(crate) fn message(self) -> String {
        let tail = match self {
            Self::Busy => "busy: the local model already has the maximum number of waiters".into(),
            Self::Timeout => "timeout: waited too long for the local model".into(),
            Self::Paused(state) => format!(
                "paused: memory pressure is {} and the local model is stopped",
                state.as_str()
            ),
            Self::Preempted => {
                "preempted: memory became critical and the request was cancelled".into()
            }
        };
        format!("{GATE_ERROR_PREFIX} {tail}")
    }

    /// Recover a gate error from a message produced by [`GateError::message`],
    /// possibly wrapped in further context.
    pub(crate) fn from_message(message: &str) -> Option<Self> {
        let at = message.find(GATE_ERROR_PREFIX)?;
        let rest = message[at + GATE_ERROR_PREFIX.len()..].trim_start();
        if rest.starts_with("busy") {
            Some(Self::Busy)
        } else if rest.starts_with("timeout") {
            Some(Self::Timeout)
        } else if rest.starts_with("preempted") {
            Some(Self::Preempted)
        } else if rest.starts_with("paused") {
            let state = if rest.contains("critical") {
                PressureState::Critical
            } else {
                PressureState::Elevated
            };
            Some(Self::Paused(state))
        } else {
            None
        }
    }
}

impl std::fmt::Display for GateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for GateError {}

/// Point-in-time view of the gate.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct GateSnapshot {
    pub(crate) active: usize,
    pub(crate) waiting: usize,
    /// Time since the last permit was released; zero while one is held.
    #[serde(serialize_with = "serialize_secs")]
    pub(crate) idle_for: Duration,
    pub(crate) paused: Option<PressureState>,
    pub(crate) acquired_total: usize,
}

fn serialize_secs<S: serde::Serializer>(value: &Duration, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(value.as_secs_f64())
}

struct Shared {
    active: AtomicUsize,
    last_activity: Mutex<Instant>,
}

/// One-permit, bounded-queue, pausable, preemptible gate.
pub(crate) struct InferenceGate {
    permits: Arc<Semaphore>,
    shared: Arc<Shared>,
    waiting: AtomicUsize,
    acquired_total: AtomicUsize,
    cancel: Mutex<CancellationToken>,
    paused: watch::Sender<Option<PressureState>>,
}

impl Default for InferenceGate {
    fn default() -> Self {
        Self::new()
    }
}

impl InferenceGate {
    pub(crate) fn new() -> Self {
        let (paused, _) = watch::channel(None);
        Self {
            permits: Arc::new(Semaphore::new(1)),
            shared: Arc::new(Shared {
                active: AtomicUsize::new(0),
                last_activity: Mutex::new(Instant::now()),
            }),
            waiting: AtomicUsize::new(0),
            acquired_total: AtomicUsize::new(0),
            cancel: Mutex::new(CancellationToken::new()),
            paused,
        }
    }

    /// Wait for the single slot, honouring `max_waiters`,
    /// `acquire_timeout_secs` and a pressure pause.
    pub(crate) async fn acquire(&self, cfg: &MlxWorkerConfig) -> Result<GatePermit, GateError> {
        let queued = self.waiting.fetch_add(1, Ordering::SeqCst);
        let _waiter = WaiterGuard(&self.waiting);
        // An uncontended caller is not a waiter; only refuse when the slot is
        // unavailable (taken, or paused) and the queue is full.
        let blocked = self.permits.available_permits() == 0 || self.paused.borrow().is_some();
        if queued >= cfg.max_waiters && blocked {
            log::debug!(
                "[mlx:gate] refusing caller: {queued} already waiting (max {})",
                cfg.max_waiters
            );
            return Err(GateError::Busy);
        }

        let timeout = Duration::from_secs(cfg.acquire_timeout_secs);
        let started = Instant::now();
        log::debug!("[mlx:gate] acquire start waiting={}", queued);
        match tokio::time::timeout(timeout, self.acquire_unpaused()).await {
            Ok(permit) => {
                let active = self.shared.active.fetch_add(1, Ordering::SeqCst) + 1;
                debug_assert!(active <= 1, "inference gate admitted {active} requests");
                self.acquired_total.fetch_add(1, Ordering::Relaxed);
                log::debug!(
                    "[mlx:gate] acquired after {}ms active={active}",
                    started.elapsed().as_millis()
                );
                Ok(GatePermit {
                    _permit: permit,
                    shared: Arc::clone(&self.shared),
                    cancel: self.cancel.lock().clone(),
                })
            }
            Err(_) => {
                let err = match *self.paused.borrow() {
                    Some(state) => GateError::Paused(state),
                    None => GateError::Timeout,
                };
                log::debug!("[mlx:gate] acquire failed after {timeout:?}: {err}");
                Err(err)
            }
        }
    }

    async fn acquire_unpaused(&self) -> OwnedSemaphorePermit {
        let mut paused = self.paused.subscribe();
        loop {
            // Wait out a pause before queueing for the slot.
            while paused.borrow_and_update().is_some() {
                if paused.changed().await.is_err() {
                    break;
                }
            }
            let permit = Arc::clone(&self.permits)
                .acquire_owned()
                .await
                .expect("the inference gate semaphore is never closed");
            // A pause may have landed while this caller queued.
            if self.paused.borrow().is_none() {
                return permit;
            }
            drop(permit);
        }
    }

    /// Cancel the in-flight request, if any. Later acquirers get a fresh token.
    pub(crate) fn preempt(&self) {
        let mut cancel = self.cancel.lock();
        cancel.cancel();
        *cancel = CancellationToken::new();
        log::info!(
            "[mlx:gate] preempted in-flight request active={}",
            self.shared.active.load(Ordering::SeqCst)
        );
    }

    /// Stop admitting new requests until [`InferenceGate::resume`].
    pub(crate) fn pause(&self, state: PressureState) {
        log::info!("[mlx:gate] paused: pressure {}", state.as_str());
        self.paused.send_replace(Some(state));
    }

    pub(crate) fn resume(&self) {
        if self.paused.send_replace(None).is_some() {
            log::info!("[mlx:gate] resumed");
        }
    }

    pub(crate) fn paused(&self) -> Option<PressureState> {
        *self.paused.borrow()
    }

    pub(crate) fn snapshot(&self) -> GateSnapshot {
        let active = self.shared.active.load(Ordering::SeqCst);
        let idle_for = if active > 0 {
            Duration::ZERO
        } else {
            self.shared.last_activity.lock().elapsed()
        };
        GateSnapshot {
            active,
            waiting: self.waiting.load(Ordering::SeqCst),
            idle_for,
            paused: self.paused(),
            acquired_total: self.acquired_total.load(Ordering::Relaxed),
        }
    }

    /// Mark activity without a request, e.g. a lazy start, so the idle timer
    /// counts from it.
    pub(crate) fn touch(&self) {
        *self.shared.last_activity.lock() = Instant::now();
    }
}

struct WaiterGuard<'a>(&'a AtomicUsize);

impl Drop for WaiterGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The inference slot. Dropping it releases the slot.
pub(crate) struct GatePermit {
    _permit: OwnedSemaphorePermit,
    shared: Arc<Shared>,
    cancel: CancellationToken,
}

impl GatePermit {
    /// Resolves when the request holding this permit is preempted.
    pub(crate) async fn cancelled(&self) {
        self.cancel.cancelled().await;
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Drop for GatePermit {
    fn drop(&mut self) {
        *self.shared.last_activity.lock() = Instant::now();
        let before = self.shared.active.fetch_sub(1, Ordering::SeqCst);
        log::debug!("[mlx:gate] released active={}", before.saturating_sub(1));
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
