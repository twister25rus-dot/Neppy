//! Pet companion: Tauri shell side (T3, §2.7 / §2.9).
//!
//! The shell owns the one thing the core cannot: the user-visible indicator.
//! It grants the core an *indicator lease* every ~2 s
//! (`openhuman.pet_companion_lease {indicator:"tray", visible}`); the core
//! only samples while a lease is live, so observation cannot outlive a
//! visible tray icon by more than the core's 6 s expiry. If there is no tray
//! (Linux, failed `setup_tray`) the shell reports `visible:false` and no
//! lease is granted.
//!
//! This module is a thin host: no webview, no JS injection, no content
//! handling. Logic lives in the core; here are the lease loop, the tray
//! presentation (`indicator`), the global hotkeys (`hotkeys`) and the RPC
//! seam (`rpc`).
//!
//! Pause is optimistic: a hotkey or tray press updates the tray *before* the
//! RPC is sent, so pause feels instant and the indicator never claims
//! observation after the user asked to stop.

mod hotkeys;
mod indicator;
mod rpc;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::AppRuntime;
use hotkeys::{
    apply_plan, desired_hotkeys, filter_conflicts, hotkey_plan, HotkeyAction, PressHandler,
    Registered, ShortcutHost, TauriShortcutHost,
};
use indicator::{indicator_for, IndicatorState, TauriTray, TrayIndicator};
use rpc::{CompanionRpc, HotkeyError, HttpCompanionRpc, LeaseRequest, LeaseResponse, Source};

pub(crate) use indicator::create_menu_items;

/// Lease cadence while the companion is enabled.
const LEASE_INTERVAL_ENABLED: Duration = Duration::from_secs(2);
/// Cadence while disabled (just enough to notice the user enabling it).
const LEASE_INTERVAL_DISABLED: Duration = Duration::from_secs(10);
/// Backoff cap when the core is unreachable.
const LEASE_BACKOFF_CAP: Duration = Duration::from_secs(10);
/// Consecutive lease failures after which the indicator is cleared: with the
/// core unreachable nothing is being observed, so nothing should be claimed.
const FAILURES_TO_CLEAR: u32 = 2;

/// Seconds between lease attempts. Exponential backoff on failure, capped.
pub(crate) fn lease_interval(enabled: bool, failures: u32) -> Duration {
    let base = if enabled {
        LEASE_INTERVAL_ENABLED
    } else {
        LEASE_INTERVAL_DISABLED
    };
    if failures == 0 {
        return base;
    }
    let factor = 1u32.checked_shl(failures.min(8)).unwrap_or(u32::MAX);
    base.saturating_mul(factor).min(LEASE_BACKOFF_CAP)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct Inner {
    indicator: IndicatorState,
    /// Last observing state shown; what an optimistic resume restores.
    last_active: IndicatorState,
    enabled: bool,
    failures: u32,
    /// False after the tray rejected the last draw; reported as `visible:false`.
    tray_ok: bool,
    registered: Registered,
    /// Accelerators that failed to register, so they are not retried every tick.
    failed: Registered,
    hotkey_errors: Vec<HotkeyError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToggleResult {
    /// Companion off, or another toggle still in flight.
    Ignored,
    /// RPC resolved; the tray shows what the core reported.
    Applied(IndicatorState),
    /// RPC failed; the tray was put back.
    Reverted,
}

pub(crate) struct Shell {
    rpc: Arc<dyn CompanionRpc>,
    tray: Arc<dyn TrayIndicator>,
    hotkeys: Arc<dyn ShortcutHost>,
    /// Ask/capture hotkeys need the macOS sensors.
    sensors_os: bool,
    inner: Mutex<Inner>,
    /// Bumped by every user toggle so a lease that started earlier cannot
    /// repaint a stale state over it.
    epoch: AtomicU64,
    toggle_pending: AtomicBool,
    ask_inflight: AtomicBool,
    capture_inflight: AtomicBool,
    me: Weak<Shell>,
}

impl Shell {
    pub(crate) fn new(
        rpc: Arc<dyn CompanionRpc>,
        tray: Arc<dyn TrayIndicator>,
        hotkeys: Arc<dyn ShortcutHost>,
        sensors_os: bool,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| Shell {
            rpc,
            tray,
            hotkeys,
            sensors_os,
            inner: Mutex::new(Inner {
                indicator: IndicatorState::Off,
                last_active: IndicatorState::Observing,
                enabled: false,
                failures: 0,
                tray_ok: true,
                registered: Registered::new(),
                failed: Registered::new(),
                hotkey_errors: Vec::new(),
            }),
            epoch: AtomicU64::new(0),
            toggle_pending: AtomicBool::new(false),
            ask_inflight: AtomicBool::new(false),
            capture_inflight: AtomicBool::new(false),
            me: me.clone(),
        })
    }

    #[cfg(test)]
    pub(crate) fn current_indicator(&self) -> IndicatorState {
        lock(&self.inner).indicator
    }

    /// Draw `state`. Returns whether the tray accepted it; on failure the
    /// indicator is recorded as off and the next lease says `visible:false`.
    fn show(&self, state: IndicatorState) -> bool {
        let spec = indicator_for(state);
        match self.tray.apply(&spec) {
            Ok(()) => {
                let mut g = lock(&self.inner);
                if g.indicator != state {
                    log::info!(
                        "[pet::companion::shell] indicator {:?} -> {:?}",
                        g.indicator,
                        state
                    );
                }
                g.indicator = state;
                g.tray_ok = true;
                if state.is_observing() {
                    g.last_active = state;
                }
                true
            }
            Err(e) => {
                log::debug!("[pet::companion::shell] tray apply failed: {e}");
                let mut g = lock(&self.inner);
                g.indicator = IndicatorState::Off;
                g.tray_ok = false;
                false
            }
        }
    }

    /// One lease round trip. Returns how long to wait before the next one.
    pub(crate) async fn lease_tick(&self) -> Duration {
        let epoch0 = self.epoch.load(Ordering::SeqCst);
        let req = {
            let g = lock(&self.inner);
            LeaseRequest {
                visible: self.tray.exists() && g.tray_ok,
                hotkey_errors: g.hotkey_errors.clone(),
            }
        };
        match self.rpc.lease(&req).await {
            Ok(resp) => {
                {
                    let mut g = lock(&self.inner);
                    g.failures = 0;
                    g.enabled = resp.enabled;
                }
                let state = IndicatorState::from_wire(
                    Some(resp.enabled),
                    &resp.state,
                    resp.paused,
                    resp.screen_capture_active,
                );
                let user_toggled = self.epoch.load(Ordering::SeqCst) != epoch0
                    || self.toggle_pending.load(Ordering::SeqCst);
                if user_toggled {
                    log::debug!("[pet::companion::shell] lease result dropped: toggle in progress");
                } else if !self.show(state) && state.is_observing() {
                    // The core thinks it is observing but we cannot draw it:
                    // withdraw the lease now rather than on the next tick.
                    log::warn!("[pet::companion::shell] cannot draw indicator; withdrawing lease");
                    let withdraw = LeaseRequest {
                        visible: false,
                        hotkey_errors: Vec::new(),
                    };
                    if let Err(e) = self.rpc.lease(&withdraw).await {
                        log::warn!("[pet::companion::shell] lease withdraw failed: {e}");
                    }
                }
                self.reconcile_hotkeys(&resp);
            }
            Err(e) => {
                let (failures, indicator) = {
                    let mut g = lock(&self.inner);
                    g.failures = g.failures.saturating_add(1);
                    (g.failures, g.indicator)
                };
                log::warn!("[pet::companion::shell] lease failed (#{failures}): {e}");
                if failures >= FAILURES_TO_CLEAR && indicator != IndicatorState::Off {
                    self.show(IndicatorState::Off);
                }
            }
        }
        let g = lock(&self.inner);
        lease_interval(g.enabled, g.failures)
    }

    fn press_handler(&self, action: HotkeyAction) -> PressHandler {
        let weak = self.me.clone();
        Arc::new(move || {
            if let Some(shell) = weak.upgrade() {
                tauri::async_runtime::spawn(async move {
                    shell.on_hotkey(action).await;
                });
            }
        })
    }

    fn reconcile_hotkeys(&self, resp: &LeaseResponse) {
        let (desired, mut errors) = desired_hotkeys(
            &resp.hotkeys,
            resp.enabled,
            self.sensors_os && resp.platform_supported,
        );
        let (mut accepted, conflicts) = filter_conflicts(
            desired,
            &self.hotkeys.dictation_current(),
            &self.hotkeys.ptt_current(),
        );
        errors.extend(conflicts);

        let (old, failed, prev_errors) = {
            let g = lock(&self.inner);
            (
                g.registered.clone(),
                g.failed.clone(),
                g.hotkey_errors.clone(),
            )
        };
        // Do not retry an accelerator that already failed until it changes.
        let mut still_failed = Registered::new();
        for (action, variants) in accepted.clone() {
            if failed.get(&action) == Some(&variants) {
                accepted.remove(&action);
                still_failed.insert(action, variants);
                if let Some(e) = prev_errors.iter().find(|e| e.name == action.name()) {
                    errors.push(e.clone());
                }
            }
        }

        let plan = hotkey_plan(&old, &accepted);
        let mut registered = old;
        let mut apply_errors = Vec::new();
        apply_plan(
            &*self.hotkeys,
            &plan,
            &|a| self.press_handler(a),
            &mut registered,
            &mut apply_errors,
        );
        for e in &apply_errors {
            if let Some(step) = plan.iter().find(|p| p.action.name() == e.name) {
                still_failed.insert(step.action, step.register.clone());
            }
        }
        errors.extend(apply_errors);

        let mut g = lock(&self.inner);
        g.registered = registered;
        g.failed = still_failed;
        g.hotkey_errors = errors;
    }

    /// Pause or resume, flipping the tray first. Shared by the hotkey and the
    /// tray menu item.
    pub(crate) async fn toggle_pause(&self, source: Source) -> ToggleResult {
        if self.toggle_pending.swap(true, Ordering::SeqCst) {
            log::debug!("[pet::companion::shell] toggle ignored: previous one in flight");
            return ToggleResult::Ignored;
        }
        let result = self.toggle_pause_inner(source).await;
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.toggle_pending.store(false, Ordering::SeqCst);
        result
    }

    async fn toggle_pause_inner(&self, source: Source) -> ToggleResult {
        let (prev, last_active) = {
            let g = lock(&self.inner);
            (g.indicator, g.last_active)
        };
        let pausing = match prev {
            IndicatorState::Off => return ToggleResult::Ignored,
            IndicatorState::Paused => false,
            _ => true,
        };
        self.epoch.fetch_add(1, Ordering::SeqCst);
        // Optimistic: the user sees the new state before the RPC is even sent.
        let optimistic = if pausing {
            IndicatorState::Paused
        } else {
            last_active
        };
        self.show(optimistic);
        log::info!(
            "[pet::companion::shell] {} requested source={}",
            if pausing { "pause" } else { "resume" },
            source.as_str()
        );
        let res = if pausing {
            self.rpc.pause(source).await
        } else {
            self.rpc.resume(source).await
        };
        match res {
            Ok(status) => {
                let state = if status.state.is_empty() {
                    optimistic
                } else {
                    IndicatorState::from_wire(
                        None,
                        &status.state,
                        status.paused,
                        status.screen_capture_active,
                    )
                };
                self.show(state);
                ToggleResult::Applied(state)
            }
            Err(e) => {
                log::warn!("[pet::companion::shell] pause/resume rpc failed: {e}; reverting");
                self.show(prev);
                ToggleResult::Reverted
            }
        }
    }

    pub(crate) async fn on_hotkey(&self, action: HotkeyAction) {
        log::debug!("[pet::companion::shell] hotkey pressed: {}", action.name());
        match action {
            HotkeyAction::Pause => {
                self.toggle_pause(Source::Hotkey).await;
            }
            HotkeyAction::Ask => {
                if self.ask_inflight.swap(true, Ordering::SeqCst) {
                    return;
                }
                match self.rpc.ask(Source::Hotkey).await {
                    Ok(o) => log::info!("[pet::companion::shell] ask -> {}", o.status),
                    Err(e) => log::warn!("[pet::companion::shell] ask failed: {e}"),
                }
                self.ask_inflight.store(false, Ordering::SeqCst);
            }
            HotkeyAction::Capture => {
                if self.capture_inflight.swap(true, Ordering::SeqCst) {
                    return;
                }
                match self.rpc.capture(Source::Hotkey).await {
                    Ok(o) => log::info!("[pet::companion::shell] capture -> {}", o.status),
                    Err(e) => log::warn!("[pet::companion::shell] capture failed: {e}"),
                }
                self.capture_inflight.store(false, Ordering::SeqCst);
            }
        }
    }

    /// Unregister every companion hotkey (shutdown).
    pub(crate) fn unregister_all(&self) {
        let registered: BTreeMap<_, _> = std::mem::take(&mut lock(&self.inner).registered);
        for variants in registered.values() {
            for v in variants {
                if let Err(e) = self.hotkeys.unregister(v) {
                    log::debug!("[pet::companion::shell] unregister {v} failed: {e}");
                }
            }
        }
    }
}

async fn run_lease_loop(shell: Arc<Shell>) {
    log::info!("[pet::companion::shell] lease loop started");
    loop {
        let wait = shell.lease_tick().await;
        tokio::time::sleep(wait).await;
    }
}

/// Managed state: the running shell and its lease task.
struct PetCompanionHandle {
    shell: Arc<Shell>,
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

/// Start the lease loop. Call after `setup_tray` (whose outcome the loop
/// discovers on its own by looking the tray icon up by id).
pub(crate) fn start(app: &AppHandle<AppRuntime>) {
    if app.try_state::<PetCompanionHandle>().is_some() {
        log::debug!("[pet::companion::shell] already started");
        return;
    }
    let shell = Shell::new(
        Arc::new(HttpCompanionRpc),
        TauriTray::new(app.clone()),
        TauriShortcutHost::new(app.clone()),
        cfg!(target_os = "macos"),
    );
    let task = tauri::async_runtime::spawn(run_lease_loop(shell.clone()));
    app.manage(PetCompanionHandle {
        shell,
        task: Mutex::new(Some(task)),
    });
    log::info!("[pet::companion::shell] started");
}

/// Abort the lease loop and release the hotkeys. The core's lease then
/// lapses on its own within its expiry.
pub(crate) fn shutdown(app: &AppHandle<AppRuntime>) {
    if let Some(h) = app.try_state::<PetCompanionHandle>() {
        if let Some(task) = lock(&h.task).take() {
            task.abort();
        }
        h.shell.unregister_all();
        log::info!("[pet::companion::shell] stopped");
    }
}

/// Tray "Pause / Resume Pet observation".
pub(crate) fn tray_toggle(app: &AppHandle<AppRuntime>) {
    let Some(h) = app.try_state::<PetCompanionHandle>() else {
        log::warn!("[pet::companion::shell] tray toggle before start");
        return;
    };
    let shell = h.shell.clone();
    tauri::async_runtime::spawn(async move {
        shell.toggle_pause(Source::Tray).await;
    });
}

/// Tray "Pet…": show the main window and ask the UI to navigate to the Now
/// tab. The UI listens for `pet-companion://navigate` (no script injection).
pub(crate) fn tray_open(app: &AppHandle<AppRuntime>) {
    if let Err(e) = crate::show_main_window(app) {
        log::warn!("[pet::companion::shell] open: show main window failed: {e}");
        return;
    }
    if let Err(e) = app.emit(
        "pet-companion://navigate",
        serde_json::json!({ "path": "/pet?tab=now" }),
    ) {
        log::warn!("[pet::companion::shell] open: emit failed: {e}");
    }
}
