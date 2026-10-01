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
//! Callers have one of two priorities. An *interactive* caller (chat, anything
//! a person is waiting on) is the default. A *background* caller, marked by
//! [`background_scope`] (the local assistant's step loop), steps aside for an
//! interactive one: it never takes the slot ahead of a waiting interactive
//! caller, and when an interactive caller has waited [`DEFAULT_YIELD_AFTER`]
//! behind a background request, that request is cancelled with
//! [`GateError::Yielded`] and its owner requeues from its checkpoint. Without
//! this a chat message waited behind a multi-minute assistant step.
//!
//! Ordering with the cron scheduler gate is always scheduler slot first, then
//! this gate, and nothing here ever waits on the scheduler, so the two cannot
//! deadlock.

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::neppy::config::schema::MlxWorkerConfig;

use super::pressure::PressureState;

tokio::task_local! {
    /// Set for the duration of a background caller's model call.
    static BACKGROUND: bool;
}

/// Run `fut` as a background caller: its gate acquisitions yield to
/// interactive ones. Task-local, so it covers only the awaits inside `fut`
/// that run on the same task, which is the shape of a model call.
pub(crate) async fn background_scope<F: Future>(fut: F) -> F::Output {
    BACKGROUND.scope(true, fut).await
}

fn is_background() -> bool {
    BACKGROUND
        .try_with(|background| *background)
        .unwrap_or(false)
}

/// How long an interactive caller waits behind a background request before it
/// is asked to yield.
pub(crate) const DEFAULT_YIELD_AFTER: Duration = Duration::from_secs(5);
/// Pause of a background caller that stepped aside, so it does not spin while
/// an interactive caller that is not yet queued catches up.
const BACKGROUND_DEFER: Duration = Duration::from_millis(20);

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
    /// A background request was cancelled so an interactive caller could run.
    /// Not a failure and not memory pressure: requeue and try again.
    Yielded,
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
            Self::Yielded => {
                "yielded: the request was cancelled so an interactive request could run".into()
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
        } else if rest.starts_with("yielded") {
            Some(Self::Yielded)
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
    /// What cancels the background request holding the slot, if one does:
    /// its token and the flag that tells its owner it was a yield.
    background: Mutex<Option<(CancellationToken, Arc<AtomicBool>)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Priority {
    Interactive,
    Background,
}

/// Counts an interactive caller while it waits.
struct InteractiveGuard<'a>(&'a AtomicUsize);

impl<'a> InteractiveGuard<'a> {
    fn new(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for InteractiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One-permit, bounded-queue, pausable, preemptible gate.
pub(crate) struct InferenceGate {
    permits: Arc<Semaphore>,
    shared: Arc<Shared>,
    waiting: AtomicUsize,
    acquired_total: AtomicUsize,
    /// Interactive callers waiting for the slot right now.
    interactive_waiting: AtomicUsize,
    yield_after: Duration,
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
                background: Mutex::new(None),
            }),
            waiting: AtomicUsize::new(0),
            acquired_total: AtomicUsize::new(0),
            interactive_waiting: AtomicUsize::new(0),
            yield_after: DEFAULT_YIELD_AFTER,
            cancel: Mutex::new(CancellationToken::new()),
            paused,
        }
    }

    /// A gate that asks a background holder to yield after `yield_after`
    /// instead of [`DEFAULT_YIELD_AFTER`]. For tests.
    #[cfg(test)]
    pub(crate) fn with_yield_after(mut self, yield_after: Duration) -> Self {
        self.yield_after = yield_after;
        self
    }

    /// Wait for the single slot, honouring `max_waiters`,
    /// `acquire_timeout_secs` and a pressure pause.
    pub(crate) async fn acquire(&self, cfg: &MlxWorkerConfig) -> Result<GatePermit, GateError> {
        let priority = if is_background() {
            Priority::Background
        } else {
            Priority::Interactive
        };
        self.acquire_as(cfg, priority).await
    }

    async fn acquire_as(
        &self,
        cfg: &MlxWorkerConfig,
        priority: Priority,
    ) -> Result<GatePermit, GateError> {
        let _interactive = (priority == Priority::Interactive)
            .then(|| InteractiveGuard::new(&self.interactive_waiting));
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
        log::debug!("[mlx:gate] acquire start waiting={queued} priority={priority:?}");
        match tokio::time::timeout(timeout, self.acquire_unpaused(priority)).await {
            Ok(permit) => {
                let active = self.shared.active.fetch_add(1, Ordering::SeqCst) + 1;
                debug_assert!(active <= 1, "inference gate admitted {active} requests");
                self.acquired_total.fetch_add(1, Ordering::Relaxed);
                log::debug!(
                    "[mlx:gate] acquired after {}ms active={active}",
                    started.elapsed().as_millis()
                );
                Ok(self.permit(permit, priority == Priority::Background, false))
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

    /// Build the permit for a slot just taken. Every permit's token is a child
    /// of the gate's, so a global [`InferenceGate::preempt`] still reaches it;
    /// a background permit's token is also registered so a yield can reach
    /// just that one.
    fn permit(
        &self,
        permit: OwnedSemaphorePermit,
        background: bool,
        maintenance: bool,
    ) -> GatePermit {
        let cancel = self.cancel.lock().child_token();
        let yielded = Arc::new(AtomicBool::new(false));
        if background {
            *self.shared.background.lock() = Some((cancel.clone(), Arc::clone(&yielded)));
        }
        GatePermit {
            _permit: permit,
            shared: Arc::clone(&self.shared),
            cancel,
            yielded,
            background,
            maintenance,
        }
    }

    async fn acquire_unpaused(&self, priority: Priority) -> OwnedSemaphorePermit {
        let mut paused = self.paused.subscribe();
        loop {
            // Wait out a pause before queueing for the slot.
            while paused.borrow_and_update().is_some() {
                if paused.changed().await.is_err() {
                    break;
                }
            }
            let permit = match priority {
                Priority::Interactive => self.take_slot_yielding().await,
                Priority::Background => Arc::clone(&self.permits)
                    .acquire_owned()
                    .await
                    .expect("the inference gate semaphore is never closed"),
            };
            // A pause may have landed while this caller queued.
            if self.paused.borrow().is_some() {
                drop(permit);
                continue;
            }
            // A background caller never keeps the slot ahead of an interactive
            // one: it lets go, and re-queues behind it.
            if priority == Priority::Background
                && self.interactive_waiting.load(Ordering::SeqCst) > 0
            {
                log::debug!("[mlx:gate] background caller steps aside for an interactive one");
                drop(permit);
                tokio::time::sleep(BACKGROUND_DEFER).await;
                continue;
            }
            return permit;
        }
    }

    /// Queue for the slot as an interactive caller. While it waits, a
    /// background request holding the slot is asked to yield every
    /// `yield_after`.
    async fn take_slot_yielding(&self) -> OwnedSemaphorePermit {
        let acquire = Arc::clone(&self.permits).acquire_owned();
        tokio::pin!(acquire);
        loop {
            tokio::select! {
                permit = &mut acquire => {
                    return permit.expect("the inference gate semaphore is never closed");
                }
                _ = tokio::time::sleep(self.yield_after) => self.yield_background(),
            }
        }
    }

    /// Cancel the background request holding the slot, if there is one.
    fn yield_background(&self) {
        if let Some((token, yielded)) = self.shared.background.lock().take() {
            log::info!(
                "[mlx:gate] an interactive caller has waited {:?}; asking the background request to yield",
                self.yield_after
            );
            yielded.store(true, Ordering::SeqCst);
            token.cancel();
        }
    }

    /// Take the slot for maintenance (unload, stop) if it is free right now,
    /// without waiting, queueing or counting as inference activity. `None`
    /// means a request holds it or is about to: skip the maintenance and try
    /// again next tick. Works while the gate is paused, since that is exactly
    /// when the worker is stopped.
    pub(crate) fn try_acquire(&self) -> Option<GatePermit> {
        let permit = Arc::clone(&self.permits).try_acquire_owned().ok()?;
        self.shared.active.fetch_add(1, Ordering::SeqCst);
        Some(self.permit(permit, false, true))
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
    /// Set when the cancel came from a yield rather than a preemption.
    yielded: Arc<AtomicBool>,
    background: bool,
    /// Held by the watchdog for an unload or stop, not by a request.
    maintenance: bool,
}

impl GatePermit {
    /// Why this permit was cancelled: a yield to an interactive caller, or a
    /// preemption (memory critical). Meaningful once it is cancelled.
    pub(crate) fn cancel_error(&self) -> GateError {
        if self.yielded.load(Ordering::SeqCst) {
            GateError::Yielded
        } else {
            GateError::Preempted
        }
    }

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
        if self.background {
            self.shared.background.lock().take();
        }
        // Maintenance is not inference: it must not restart the idle clock, or
        // an unload would postpone the stop that follows it.
        if !self.maintenance {
            *self.shared.last_activity.lock() = Instant::now();
        }
        let before = self.shared.active.fetch_sub(1, Ordering::SeqCst);
        log::debug!("[mlx:gate] released active={}", before.saturating_sub(1));
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
