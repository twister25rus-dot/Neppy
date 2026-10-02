//! The clipboard and autonomous-screen steps of a sample (see `observer`).
//! Same rules: re-check the gate after every sensor call, scrub before an
//! event exists, never keep an image.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::metrics::Metrics;
use super::observer::{event, note_drop, on_sensor_error, scrub_or_drop};
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
        let ask = {
            let mut g = rt.lock();
            g.sample.screen_active = false;
            !std::mem::replace(&mut g.sample.screen_requested, true)
        };
        if ask {
            log::info!("[pet::companion] screen capture needs Screen Recording: requesting once");
            rt.sensor.request_screen_recording();
        }
        return;
    }
    let now = rt.clock.instant();
    let due = {
        let mut g = rt.lock();
        g.sample.screen_active = true;
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
        Err(SensorError::PermissionRequired) => rt.lock().sample.screen_active = false,
        Err(e) => on_sensor_error(rt, &e),
    }
}
