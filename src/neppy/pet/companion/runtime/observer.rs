//! The sampler: a dedicated `std::thread` (`pet-companion-sampler`) that runs
//! only while the companion is enabled, and samples only while the gate is
//! open (enabled, not paused, fresh indicator lease, supported platform).
//!
//! One sample, in order (PC9 / PC10):
//! 1. app identity → always-excluded / user-excluded app? stop here: the title
//!    and selection are never requested for an excluded app;
//! 2. title + selection (selection only if not a secure field) → title rule →
//!    scrub → app-switch / title-change event on change;
//! 3. selection on change → scrub (drop secrets) → event;
//! 4. clipboard on `changeCount` change → concealed? drop → scrub → event
//!    (switches to on-demand reads if macOS asks before each paste);
//! 5. autonomous screen capture (Screen Recording requested once, on first
//!    need) on window change or every `screen_min_interval_secs`, OCR only if
//!    the frame changed → scrub → event. The image never leaves the sensor.
//!
//! Every text passes through `sensitive::scrub` before an event exists, and
//! the gate is re-checked after every sensor call: a result that arrives after
//! a pause is discarded.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::metrics::Metrics;
use super::pipeline;
use super::sensor::AppIdentity;
use super::state::Runtime;
use crate::neppy::desktop::accessibility::SensorError;
use crate::neppy::pet::companion::sensitive::{scrub, ScrubCtx, ScrubResult, MAX_SELECTION_CHARS};
use crate::neppy::pet::companion::types::{
    DropReason, ObservationEvent, ObservationFlags, ObservationKind,
};

/// Never observed, whatever the user's lists say (the lock screen).
pub const ALWAYS_EXCLUDED_BUNDLES: &[&str] = &["com.apple.loginwindow"];
const IDLE_SLOW_SECS: f64 = 60.0;

/// Start the sampler thread if the companion is enabled and none is running.
pub fn ensure_sampler(rt: &Arc<Runtime>) {
    if !rt.is_enabled() || rt.sampler_running() {
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let weak = Arc::downgrade(rt);
    let stop_t = stop.clone();
    let spawned = std::thread::Builder::new()
        .name("pet-companion-sampler".into())
        .spawn(move || run(weak, stop_t));
    match spawned {
        Ok(handle) => {
            rt.set_sampler(handle.thread().clone(), stop);
            log::info!("[pet::companion] sampler started");
        }
        Err(e) => log::warn!("[pet::companion] could not start sampler: {e}"),
    }
}

fn run(rt: std::sync::Weak<Runtime>, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Some(rt) = rt.upgrade() else { break };
        let wait = if rt.gate().is_err() {
            rt.timing.parked
        } else {
            let t0 = Instant::now();
            sample_once(&rt);
            rt.metrics
                .record_sample(t0.elapsed().as_millis().min(u32::MAX as u128) as u32);
            if rt.lock().sample.idle_secs > IDLE_SLOW_SECS {
                rt.timing.idle
            } else {
                rt.timing.active
            }
        };
        drop(rt);
        std::thread::park_timeout(wait);
    }
    log::info!("[pet::companion] sampler stopped");
}

pub(super) fn hash_of<T: Hash>(v: T) -> u64 {
    let mut h = DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

pub(super) fn app_key(app: &AppIdentity) -> String {
    app.bundle_id
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_else(|| app.app_name.trim().to_lowercase())
}

/// Count a drop and show it in "what the pet sees" once per distinct `key`.
pub(super) fn note_drop(
    rt: &Runtime,
    kind: ObservationKind,
    app: &AppIdentity,
    reason: DropReason,
    key: &str,
) {
    let now = rt.clock.utc();
    let mut g = rt.lock();
    let key = format!("{}|{key}", reason.as_str());
    if g.sample.last_drop.as_deref() == Some(key.as_str()) {
        return;
    }
    g.sample.last_drop = Some(key);
    g.buffer.record_drop(now, kind, &app.app_name, reason);
    drop(g);
    rt.metrics.record_drop(reason);
    log::debug!(
        "[pet::companion] dropped kind={} reason={} bundle={:?}",
        kind.as_str(),
        reason.as_str(),
        app.bundle_id
    );
}

pub(super) fn on_sensor_error(rt: &Runtime, e: &SensorError) {
    let mut g = rt.lock();
    match e {
        SensorError::PermissionMissing => g.sample.permission_missing = true,
        SensorError::HelperUnavailable(_) => {
            g.sample.helper_unavailable = true;
            Metrics::inc(&rt.metrics.helper_errors);
        }
        SensorError::NoFocusedApp | SensorError::TargetChanged | SensorError::Timeout => {}
        _ => {
            g.sample.sensor_error = true;
            Metrics::inc(&rt.metrics.sensor_errors);
        }
    }
    log::debug!("[pet::companion] sensor error: {e}");
}

pub(super) fn event(
    rt: &Runtime,
    kind: ObservationKind,
    app: &AppIdentity,
    title: Option<&ScrubResult>,
    text: Option<ScrubResult>,
) -> ObservationEvent {
    let redacted = title.is_some_and(ScrubResult::was_redacted)
        || text.as_ref().is_some_and(ScrubResult::was_redacted);
    ObservationEvent {
        at: rt.clock.utc(),
        kind,
        app_name: app.app_name.clone(),
        bundle_id: app.bundle_id.clone(),
        title: title.and_then(|t| t.text().cloned()),
        text: text.and_then(ScrubResult::into_text),
        flags: ObservationFlags {
            redacted,
            user_initiated: false,
        },
    }
}

/// Scrub `raw` for `kind`; a hard drop is counted and returns `None`.
pub(super) fn scrub_or_drop(
    rt: &Runtime,
    raw: &str,
    ctx: &ScrubCtx,
    kind: ObservationKind,
    app: &AppIdentity,
) -> Option<ScrubResult> {
    let r = scrub(raw, ctx);
    if let Some(k) = r.drop_kind() {
        note_drop(rt, kind, app, k.drop_reason(), &hash_of(raw).to_string());
        return None;
    }
    Some(r)
}

/// One sample. Public for tests (they drive it without the thread).
pub fn sample_once(rt: &Arc<Runtime>) {
    if !rt.observing_allowed() {
        return;
    }
    let (settings, excl) = rt.settings_and_exclusions();
    if !settings.sources.any_on() {
        return;
    }

    // 1. App identity only.
    let app = match rt.sensor.frontmost_app() {
        Ok(a) => a,
        Err(e) => return on_sensor_error(rt, &e),
    };
    if !rt.observing_allowed() {
        return;
    }
    {
        let mut g = rt.lock();
        g.sample.permission_missing = false;
        g.sample.sensor_error = false;
        g.sample.idle_secs = app.idle_secs;
    }
    let key = app_key(&app);
    let excluded = ALWAYS_EXCLUDED_BUNDLES
        .iter()
        .any(|b| *b == key)
        .then_some(DropReason::ExcludedApp)
        .or_else(|| excl.check_app(app.bundle_id.as_deref(), &app.app_name));
    if let Some(reason) = excluded {
        set_excluded(rt, true);
        return note_drop(rt, ObservationKind::AppSwitch, &app, reason, &key);
    }

    // 2. Title (+ selection unless a secure field is focused).
    let want_selection = settings.sources.selection && !app.is_secure_field;
    let ctx = match rt
        .sensor
        .frontmost_context(want_selection, MAX_SELECTION_CHARS)
    {
        Ok(c) => c,
        Err(e) => return on_sensor_error(rt, &e),
    };
    if !rt.observing_allowed() || ctx.pid != app.pid {
        return;
    }
    let raw_title = ctx.window_title.unwrap_or_default();
    if let Some(reason) = excl.check_title(&raw_title) {
        set_excluded(rt, true);
        return note_drop(
            rt,
            ObservationKind::TitleChange,
            &app,
            reason,
            &format!("{key}|{}", hash_of(&raw_title)),
        );
    }
    set_excluded(rt, false);
    let title = scrub_or_drop(
        rt,
        &raw_title,
        &ScrubCtx::title(),
        ObservationKind::TitleChange,
        &app,
    );
    let title_text = title
        .as_ref()
        .and_then(|t| t.text())
        .map(|t| t.as_str().to_string());
    let window_hash = hash_of((app.pid, &key, &title_text));
    let (window_changed, app_changed) = {
        let mut g = rt.lock();
        let changed = g.sample.last_window != Some(window_hash);
        let app_changed = g.sample.last_app_key.as_deref() != Some(key.as_str());
        if changed {
            g.sample.last_window = Some(window_hash);
            g.sample.last_drop = None;
        }
        if app_changed {
            g.sample.last_app_key = Some(key.clone());
            g.sample.app_since = Some(rt.clock.instant());
            g.sample.last_selection = None;
        }
        (changed, app_changed)
    };
    if window_changed && settings.sources.app_window {
        let kind = if app_changed {
            ObservationKind::AppSwitch
        } else {
            ObservationKind::TitleChange
        };
        pipeline::process_event(rt, event(rt, kind, &app, title.as_ref(), None));
    }

    // 3. Selection.
    let secure = app.is_secure_field || ctx.is_secure_field;
    if settings.sources.selection && secure {
        note_drop(
            rt,
            ObservationKind::Selection,
            &app,
            DropReason::SecureField,
            &key,
        );
    } else if settings.sources.selection {
        if let Some(sel) = ctx.selected_text.filter(|s| !s.trim().is_empty()) {
            let h = hash_of(&sel);
            let fresh = {
                let mut g = rt.lock();
                let fresh = g.sample.last_selection != Some(h);
                g.sample.last_selection = Some(h);
                fresh
            };
            if fresh {
                if let Some(r) = scrub_or_drop(
                    rt,
                    &sel,
                    &ScrubCtx::selection(false),
                    ObservationKind::Selection,
                    &app,
                ) {
                    pipeline::process_event(
                        rt,
                        event(
                            rt,
                            ObservationKind::Selection,
                            &app,
                            title.as_ref(),
                            Some(r),
                        ),
                    );
                }
            }
        }
    }

    // 4. Clipboard.
    if settings.sources.clipboard && !rt.lock().sample.clipboard_on_demand {
        super::sources::clipboard_step(rt, &app, title.as_ref());
    }

    // 5. Screen.
    if settings.sources.screen_capture && !secure {
        let interval = Duration::from_secs(settings.screen_min_interval_secs.max(1) as u64);
        super::sources::screen_step(
            rt,
            &app,
            title.as_ref(),
            window_changed,
            interval,
            &settings.ocr_languages,
        );
    } else {
        super::sources::set_screen_active(rt, false);
    }
}

fn set_excluded(rt: &Runtime, excluded: bool) {
    let was = {
        let mut g = rt.lock();
        let was = g.sample.excluded;
        g.sample.excluded = excluded;
        if excluded {
            g.sample.screen_active = false;
            g.sample.last_window = None;
        }
        was
    };
    if was != excluded {
        rt.sensor.screen_reset();
    }
}
