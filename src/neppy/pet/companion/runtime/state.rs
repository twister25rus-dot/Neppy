//! The companion's shared runtime state: the indicator lease, pause, the
//! enabled flag, cached settings, the in-memory buffer and rate limiter.
//!
//! Invariants (PC8 / PC12):
//! * nothing is sampled unless `enabled && !paused && lease_valid &&
//!   platform_supported` ([`Runtime::gate`]); the sampler re-checks the gate
//!   before AND after every sensor call and discards a result if it flipped;
//! * a lease lasts [`LEASE_TTL`] (6 s); only the shell grants one, every 2 s,
//!   and only while its indicator is visible;
//! * pause flips the atomic FIRST, then cancels in-flight generation, clears
//!   the buffer and persists `paused` / `paused_until` (survives restarts).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;

pub use super::clock::{Clock, SystemClock, Timing};
pub use super::persist::{load_pause, save_pause};

use super::bus::{self, CompanionUiEvent};
use super::generate::Generator;
use super::handoff::HandoffRunner;
use super::metrics::Metrics;
use super::sensor::Sensor;
use crate::neppy::config::Config;
use crate::neppy::pet::companion::buffer::ObservationBuffer;
use crate::neppy::pet::companion::exclusions::Exclusions;
use crate::neppy::pet::companion::ratelimit::RateLimiter;
use crate::neppy::pet::companion::settings::CompanionSettings;
use crate::neppy::pet::companion::store;

/// A lease from the shell lasts this long (the shell renews every 2 s).
pub const LEASE_TTL: Duration = Duration::from_secs(6);

/// Why observation is not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suspend {
    Off,
    Paused,
    Unsupported,
    NoIndicator,
}

/// Per-sampler bookkeeping (hashes only, never text).
#[derive(Debug, Default)]
pub struct SampleState {
    pub last_window: Option<u64>,
    pub last_app_key: Option<String>,
    pub app_since: Option<Instant>,
    pub last_drop: Option<String>,
    pub last_selection: Option<u64>,
    pub clip_baseline: Option<i64>,
    pub clipboard_on_demand: bool,
    pub last_screen: Option<Instant>,
    pub screen_requested: bool,
    pub screen_active: bool,
    pub excluded: bool,
    pub idle_secs: f64,
    pub helper_unavailable: bool,
    pub permission_missing: bool,
    pub sensor_error: bool,
}

pub struct Inner {
    pub config: Option<Config>,
    pub config_loaded_at: Option<Instant>,
    pub settings: Arc<CompanionSettings>,
    pub exclusions: Arc<Exclusions>,
    pub buffer: ObservationBuffer,
    pub ratelimit: RateLimiter,
    pub paused_at: Option<DateTime<Utc>>,
    pub paused_until: Option<DateTime<Utc>>,
    pub cancel: CancellationToken,
    pub handoffs: HashMap<String, tokio::task::AbortHandle>,
    pub sample: SampleState,
}

struct SamplerHandle {
    thread: std::thread::Thread,
    stop: Arc<AtomicBool>,
}

/// The companion runtime. One global instance in production
/// (`runtime::global()`); tests build their own with fakes.
pub struct Runtime {
    pub sensor: Arc<dyn Sensor>,
    pub generator: Arc<dyn Generator>,
    pub handoff: Arc<dyn HandoffRunner>,
    pub clock: Arc<dyn Clock>,
    pub timing: Timing,
    pub metrics: Metrics,
    pub(crate) inner: Mutex<Inner>,
    enabled: AtomicBool,
    paused: AtomicBool,
    lease_until: Mutex<Option<Instant>>,
    sampler: Mutex<Option<SamplerHandle>>,
    tokio: Option<tokio::runtime::Handle>,
}

fn compile(settings: &CompanionSettings) -> Arc<Exclusions> {
    Arc::new(Exclusions::from_settings(settings))
}

impl Runtime {
    pub fn new(
        sensor: Arc<dyn Sensor>,
        generator: Arc<dyn Generator>,
        handoff: Arc<dyn HandoffRunner>,
        clock: Arc<dyn Clock>,
        timing: Timing,
    ) -> Arc<Self> {
        let settings = CompanionSettings::default();
        Arc::new(Self {
            sensor,
            generator,
            handoff,
            clock,
            timing,
            metrics: Metrics::default(),
            inner: Mutex::new(Inner {
                config: None,
                config_loaded_at: None,
                exclusions: compile(&settings),
                settings: Arc::new(settings),
                buffer: ObservationBuffer::new(),
                ratelimit: RateLimiter::new(),
                paused_at: None,
                paused_until: None,
                cancel: CancellationToken::new(),
                handoffs: HashMap::new(),
                sample: SampleState::default(),
            }),
            enabled: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            lease_until: Mutex::new(None),
            sampler: Mutex::new(None),
            tokio: tokio::runtime::Handle::try_current().ok(),
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The async runtime to spawn background work on: the caller's when there
    /// is one, else the runtime the companion was created on.
    pub fn tokio(&self) -> Option<tokio::runtime::Handle> {
        tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.tokio.clone())
    }

    // ---- config / settings ----

    /// Bind to `config`'s workspace. On a workspace change (login, logout, a
    /// test with a fresh HOME) all observation state is reset and settings and
    /// pause state are reloaded from that workspace's `companion.db`.
    pub fn bind(self: &Arc<Self>, config: &Config) -> Result<()> {
        let same = self
            .lock()
            .config
            .as_ref()
            .is_some_and(|c| c.workspace_dir == config.workspace_dir);
        if same {
            let mut g = self.lock();
            g.config = Some(config.clone());
            g.config_loaded_at = Some(self.clock.instant());
            return Ok(());
        }
        let settings = store::load_settings(config)?;
        let (paused, until) = load_pause(config)?;
        let now = self.clock.utc();
        let history = store::recent_proactive(config, now - chrono::Duration::hours(1))?;
        log::info!(
            "[pet::companion] bound to workspace enabled={} paused={paused}",
            settings.enabled
        );
        {
            let mut g = self.lock();
            g.cancel.cancel();
            g.cancel = CancellationToken::new();
            g.config = Some(config.clone());
            g.config_loaded_at = Some(self.clock.instant());
            g.exclusions = compile(&settings);
            g.settings = Arc::new(settings.clone());
            g.buffer.clear();
            g.ratelimit = RateLimiter::from_history(&history);
            g.paused_at = paused.then_some(now);
            g.paused_until = until;
            g.sample = SampleState::default();
        }
        *self.lease_until.lock().unwrap_or_else(|p| p.into_inner()) = None;
        self.paused.store(paused, Ordering::SeqCst);
        self.sensor.screen_reset();
        self.set_enabled(settings.enabled);
        Ok(())
    }

    pub fn config(&self) -> Option<Config> {
        self.lock().config.clone()
    }

    /// Seconds since the config was last (re)loaded, for the lease fast path.
    pub fn config_age(&self) -> Option<Duration> {
        self.lock()
            .config_loaded_at
            .map(|t| self.clock.instant().saturating_duration_since(t))
    }

    pub fn settings(&self) -> Arc<CompanionSettings> {
        self.lock().settings.clone()
    }

    pub fn settings_and_exclusions(&self) -> (Arc<CompanionSettings>, Arc<Exclusions>) {
        let g = self.lock();
        (g.settings.clone(), g.exclusions.clone())
    }

    /// Install new settings (after a validated update).
    pub fn apply_settings(self: &Arc<Self>, settings: CompanionSettings) {
        let exclusions_changed;
        {
            let mut g = self.lock();
            exclusions_changed = g.settings.excluded_apps != settings.excluded_apps
                || g.settings.excluded_title_patterns != settings.excluded_title_patterns;
            if exclusions_changed {
                g.exclusions = compile(&settings);
                g.sample.last_drop = None;
                g.sample.last_window = None;
            }
            g.settings = Arc::new(settings.clone());
        }
        if exclusions_changed {
            self.sensor.screen_reset();
        }
        self.set_enabled(settings.enabled);
        self.publish_state();
    }

    // ---- enabled / lease / pause ----

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn set_enabled(self: &Arc<Self>, enabled: bool) {
        let was = self.enabled.swap(enabled, Ordering::SeqCst);
        if enabled && !was {
            log::info!("[pet::companion] enabled");
            self.sensor.warm_up();
            super::observer::ensure_sampler(self);
        } else if !enabled {
            if was {
                log::info!("[pet::companion] disabled");
            }
            self.stop_sampler();
            let mut g = self.lock();
            g.cancel.cancel();
            g.cancel = CancellationToken::new();
            g.buffer.clear();
            g.sample = SampleState::default();
        }
    }

    pub fn lease_valid(&self) -> bool {
        self.lease_until
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_some_and(|until| self.clock.instant() < until)
    }

    /// Record a lease from the shell. `visible = false` revokes it at once.
    pub fn grant_lease(&self, visible: bool) {
        let was_valid = self.lease_valid();
        {
            let mut l = self.lease_until.lock().unwrap_or_else(|p| p.into_inner());
            *l = visible.then(|| self.clock.instant() + LEASE_TTL);
        }
        if visible && !was_valid {
            log::info!("[pet::companion] indicator lease gained");
            self.wake_sampler();
        } else if !visible && was_valid {
            log::info!("[pet::companion] indicator lease revoked (indicator not visible)");
        }
    }

    /// Paused, resuming automatically once `paused_until` has passed.
    pub fn is_paused(self: &Arc<Self>) -> bool {
        if !self.paused.load(Ordering::SeqCst) {
            return false;
        }
        let expired = self
            .lock()
            .paused_until
            .is_some_and(|u| self.clock.utc() >= u);
        if expired {
            self.resume("timer");
            return false;
        }
        true
    }

    pub fn paused_at(&self) -> Option<DateTime<Utc>> {
        self.lock().paused_at
    }

    pub fn paused_until(&self) -> Option<DateTime<Utc>> {
        self.lock().paused_until
    }

    pub fn pause(self: &Arc<Self>, minutes: Option<u32>, source: &str) {
        // The flag flips first: the sampler checks it around every sensor call.
        self.paused.store(true, Ordering::SeqCst);
        let now = self.clock.utc();
        let until = minutes.map(|m| now + chrono::Duration::minutes(m as i64));
        let config = {
            let mut g = self.lock();
            g.paused_at = Some(now);
            g.paused_until = until;
            g.cancel.cancel();
            g.cancel = CancellationToken::new();
            g.buffer.clear();
            g.sample.screen_active = false;
            g.config.clone()
        };
        if let Some(c) = config {
            if let Err(e) = save_pause(&c, true, until) {
                log::warn!("[pet::companion] could not persist pause: {e:#}");
            }
        }
        log::info!(
            "[pet::companion] paused source={source} timed={}",
            until.is_some()
        );
        self.publish_state();
    }

    pub fn resume(self: &Arc<Self>, source: &str) {
        let config = {
            let mut g = self.lock();
            g.paused_at = None;
            g.paused_until = None;
            // Re-baseline: nothing copied or shown during the pause is read.
            g.sample.clip_baseline = None;
            g.sample.last_selection = None;
            g.sample.last_screen = None;
            g.config.clone()
        };
        self.sensor.screen_reset();
        self.paused.store(false, Ordering::SeqCst);
        if let Some(c) = config {
            if let Err(e) = save_pause(&c, false, None) {
                log::warn!("[pet::companion] could not persist resume: {e:#}");
            }
        }
        log::info!("[pet::companion] resumed source={source}");
        self.wake_sampler();
        self.publish_state();
    }

    /// The observation gate (see the module docs).
    pub fn gate(self: &Arc<Self>) -> Result<(), Suspend> {
        if !self.is_enabled() {
            return Err(Suspend::Off);
        }
        if self.is_paused() {
            return Err(Suspend::Paused);
        }
        if !self.sensor.platform_supported() {
            return Err(Suspend::Unsupported);
        }
        if !self.lease_valid() {
            return Err(Suspend::NoIndicator);
        }
        Ok(())
    }

    pub fn observing_allowed(self: &Arc<Self>) -> bool {
        self.gate().is_ok()
    }

    /// `(state, suspended_reason)` for status / lease / socket.
    pub fn state(self: &Arc<Self>) -> (&'static str, Option<&'static str>) {
        match self.gate() {
            Err(Suspend::Off) => ("off", None),
            Err(Suspend::Paused) => ("paused", None),
            Err(Suspend::Unsupported) => ("suspended", Some("unsupported_platform")),
            Err(Suspend::NoIndicator) => ("suspended", Some("no_indicator")),
            Ok(()) => {
                let s = &self.lock().sample;
                if s.permission_missing {
                    ("suspended", Some("permission_missing"))
                } else if s.sensor_error {
                    ("suspended", Some("sensor_error"))
                } else {
                    ("observing", None)
                }
            }
        }
    }

    /// True only while autonomous capture is armed: the source is on, Screen
    /// Recording is granted, and we are observing a non-excluded app.
    pub fn screen_capture_active(self: &Arc<Self>) -> bool {
        self.state().0 == "observing" && {
            let g = self.lock();
            g.settings.sources.screen_capture && g.sample.screen_active && !g.sample.excluded
        }
    }

    pub fn publish_state(self: &Arc<Self>) {
        let (state, reason) = self.state();
        bus::publish(CompanionUiEvent::State {
            state,
            paused: state == "paused",
            suspended_reason: reason,
            screen_capture_active: self.screen_capture_active(),
        });
    }

    // ---- sampler thread ----

    pub(crate) fn set_sampler(&self, thread: std::thread::Thread, stop: Arc<AtomicBool>) {
        *self.sampler.lock().unwrap_or_else(|p| p.into_inner()) =
            Some(SamplerHandle { thread, stop });
    }

    pub(crate) fn sampler_running(&self) -> bool {
        self.sampler
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|h| !h.stop.load(Ordering::SeqCst))
    }

    pub fn wake_sampler(&self) {
        if let Some(h) = self
            .sampler
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
        {
            h.thread.unpark();
        }
    }

    pub fn stop_sampler(&self) {
        if let Some(h) = self
            .sampler
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            h.stop.store(true, Ordering::SeqCst);
            h.thread.unpark();
            log::info!("[pet::companion] sampler stopping");
        }
    }

    /// Cancel any in-flight generation and abort running hand-offs.
    pub fn cancel_all_work(&self) {
        let mut g = self.lock();
        g.cancel.cancel();
        g.cancel = CancellationToken::new();
        for (_, h) in g.handoffs.drain() {
            h.abort();
        }
    }
}
