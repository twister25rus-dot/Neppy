//! The clipboard and autonomous-screen steps of a sample (see `observer`).
//! Same rules: re-check the gate after every sensor call, scrub before an
//! event exists, never keep an image.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::metrics::Metrics;
use super::observer::{
    app_key, event, note_drop, on_sensor_error, scrub_or_drop, ALWAYS_EXCLUDED_BUNDLES,
};
use super::pipeline;
use super::sensor::{AppIdentity, ScreenSample};
use super::state::Runtime;
use crate::neppy::desktop::accessibility::{PermissionState, SensorError};
use crate::neppy::pet::companion::sensitive::{ScrubCtx, ScrubResult, MAX_CLIPBOARD_CHARS};
use crate::neppy::pet::companion::types::{DropReason, ObservationEvent, ObservationKind};

pub(super) fn clipboard_step(rt: &Arc<Runtime>, app: &AppIdentity, title: Option<&ScrubResult>) {
    let peek = match rt.sensor.clipboard_peek() {
        Ok(p) => p,
        Err(e) => return on_sensor_error(rt, &e),
    };
    if matches!(
        peek.access_behavior.as_deref(),
        Some("ask") | Some("default")
    ) {
        log::info!(
            "[pet::companion] pasteboard asks before reads: clipboard switched to on-demand"
        );
        rt.lock().sample.clipboard_on_demand = true;
        return;
    }
    let changed = {
        let mut g = rt.lock();
        let prev = g.sample.clip_baseline.replace(peek.change_count);
        prev.is_some_and(|p| p != peek.change_count)
    };
    if !changed || !rt.observing_allowed() {
        return;
    }
    clipboard_read_event(rt, app, title, false);
}

/// Read the clipboard now and turn it into an event (sampler or Ask).
pub(crate) fn clipboard_read_event(
    rt: &Arc<Runtime>,
    app: &AppIdentity,
    title: Option<&ScrubResult>,
    user_initiated: bool,
) -> Option<ObservationEvent> {
    let read = match rt.sensor.clipboard_read(MAX_CLIPBOARD_CHARS) {
        Ok(r) => r,
        Err(e) => {
            on_sensor_error(rt, &e);
            return None;
        }
    };
    if !rt.observing_allowed() {
        return None;
    }
    if read.sensitive {
        note_drop(
            rt,
            ObservationKind::Clipboard,
            app,
            DropReason::ConcealedClipboard,
            &read.change_count.to_string(),
        );
        return None;
    }
    let text = read.text.filter(|t| !t.trim().is_empty())?;
    let r = scrub_or_drop(
        rt,
        &text,
        &ScrubCtx::clipboard(false),
        ObservationKind::Clipboard,
        app,
    )?;
    let mut ev = event(rt, ObservationKind::Clipboard, app, title, Some(r));
    ev.flags.user_initiated = user_initiated;
    if user_initiated {
        return Some(ev);
    }
    pipeline::process_event(rt, ev);
    None
}

pub(super) fn screen_step(
    rt: &Arc<Runtime>,
    app: &AppIdentity,
    title: Option<&ScrubResult>,
    window_changed: bool,
    interval: Duration,
    languages: &[String],
) {
    if rt.sensor.screen_recording() != PermissionState::Granted {
        set_screen_active(rt, false);
        let ask = !std::mem::replace(&mut rt.lock().sample.screen_requested, true);
        if ask {
            log::info!("[pet::companion] screen capture needs Screen Recording: requesting once");
            rt.sensor.request_screen_recording();
        }
        return;
    }
    // Report capture as armed BEFORE the first frame is taken, and give the
    // indicator one sample interval to show "observing screen" before any
    // capture happens: arming publishes the state and returns.
    if set_screen_active(rt, true) {
        log::info!("[pet::companion] screen capture armed: first capture on the next sample");
        return;
    }
    let now = rt.clock.instant();
    let due = {
        let mut g = rt.lock();
        let since = g
            .sample
            .last_screen
            .map(|t| now.saturating_duration_since(t));
        let due = match since {
            None => true,
            Some(s) if window_changed => s >= rt.timing.capture_floor,
            Some(s) => s >= interval,
        };
        if due {
            g.sample.last_screen = Some(now);
        }
        due
    };
    if !due || !rt.observing_allowed() {
        return;
    }
    let t0 = Instant::now();
    let result = rt
        .sensor
        .screen_sample(Some(app.pid), window_changed, languages);
    Metrics::add(
        &rt.metrics.capture_ms_total,
        t0.elapsed().as_millis() as u64,
    );
    if !rt.observing_allowed() {
        return;
    }
    match result {
        Ok(ScreenSample::Unchanged) => Metrics::inc(&rt.metrics.screen_unchanged),
        Ok(ScreenSample::Text { text, ocr_ms }) => {
            Metrics::inc(&rt.metrics.ocr_calls);
            Metrics::add(&rt.metrics.ocr_ms_total, ocr_ms);
            if text.trim().is_empty() {
                return;
            }
            // OCR takes long enough for the frontmost window to change (another
            // app, an excluded tab, a secure field). Re-check before the text
            // becomes an event; drop it when the frame may not be this app's.
            if let Err(reason) = still_on_target(rt, app) {
                drop(text);
                rt.sensor.screen_reset();
                match reason {
                    Some(reason) => {
                        note_drop(rt, ObservationKind::Capture, app, reason, "screen-late")
                    }
                    None => {
                        log::debug!(
                            "[pet::companion] frontmost window changed during OCR: capture dropped"
                        );
                    }
                }
                return;
            }
            if let Some(r) =
                scrub_or_drop(rt, &text, &ScrubCtx::ocr(), ObservationKind::Capture, app)
            {
                pipeline::process_event(
                    rt,
                    event(rt, ObservationKind::Capture, app, title, Some(r)),
                );
            }
        }
        Err(SensorError::SecureFieldFocused) => {
            note_drop(
                rt,
                ObservationKind::Capture,
                app,
                DropReason::SecureField,
                "screen",
            );
        }
        Err(SensorError::PermissionRequired) => {
            set_screen_active(rt, false);
        }
        Err(e) => on_sensor_error(rt, &e),
    }
}

/// Set whether autonomous screen capture is armed; publishes the companion
/// state on a change so the indicator shows "observing screen" as soon as
/// capture is armed (and stops showing it as soon as it is not). Returns `true`
/// when the flag changed.
pub(super) fn set_screen_active(rt: &Arc<Runtime>, active: bool) -> bool {
    let changed = {
        let mut g = rt.lock();
        std::mem::replace(&mut g.sample.screen_active, active) != active
    };
    if changed {
        rt.publish_state();
    }
    changed
}

/// Whether the frontmost window is still the one a screen sample was taken
/// for, re-read after OCR. `Err(Some(reason))` when it is now excluded or a
/// secure field; `Err(None)` when the app changed or the check itself failed
/// (fail closed: the text is dropped either way).
fn still_on_target(rt: &Arc<Runtime>, app: &AppIdentity) -> Result<(), Option<DropReason>> {
    let now = rt.sensor.frontmost_app().map_err(|_| None)?;
    let key = app_key(&now);
    if now.pid != app.pid || key != app_key(app) {
        return Err(None);
    }
    let (_, excl) = rt.settings_and_exclusions();
    if ALWAYS_EXCLUDED_BUNDLES.contains(&key.as_str()) {
        return Err(Some(DropReason::ExcludedApp));
    }
    if let Some(reason) = excl.check_app(now.bundle_id.as_deref(), &now.app_name) {
        return Err(Some(reason));
    }
    if now.is_secure_field {
        return Err(Some(DropReason::SecureField));
    }
    let ctx = rt.sensor.frontmost_context(false, 0).map_err(|_| None)?;
    if ctx.pid != app.pid {
        return Err(None);
    }
    if ctx.is_secure_field {
        return Err(Some(DropReason::SecureField));
    }
    match excl.check_title(ctx.window_title.as_deref().unwrap_or_default()) {
        Some(reason) => Err(Some(reason)),
        None => Ok(()),
    }
}
