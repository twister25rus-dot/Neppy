//! RPC operations for `openhuman.pet_companion_*` (wire contract: plan §2.9 +
//! the shell / UI contract). Each takes the runtime explicitly so tests can
//! drive a runtime built from fakes. Errors are field-named strings.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::actions::{self, ActResult};
use super::metrics::Metrics;
use super::observer::ALWAYS_EXCLUDED_BUNDLES;
use super::pipeline;
use super::sensor::RegionSample;
use super::sources::clipboard_read_event;
use super::state::{Runtime, Suspend};
use crate::neppy::config::Config;
use crate::neppy::desktop::accessibility::{PermissionState, SensorError};
use crate::neppy::pet::companion::policy;
use crate::neppy::pet::companion::sensitive::{scrub, ScrubCtx, MAX_SELECTION_CHARS};
use crate::neppy::pet::companion::settings::{CompanionSettings, CompanionSettingsPatch};
use crate::neppy::pet::companion::store::{self, UpdateError};
use crate::neppy::pet::companion::types::*;

pub const RECENT_LIMIT: usize = 20;
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn settings_json(s: &CompanionSettings) -> Value {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.insert("unavailable_sources".into(), json!(UNAVAILABLE_SOURCES));
    }
    v
}

pub fn get(rt: &Arc<Runtime>, config: &Config) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    Ok(settings_json(&rt.settings()))
}

/// Apply a flat patch (`CompanionSettingsPatch`, unknown fields refused).
pub fn update(
    rt: &Arc<Runtime>,
    config: &Config,
    params: Map<String, Value>,
) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    let patch: CompanionSettingsPatch =
        serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid patch: {e}"))?;
    let next = store::update_settings(config, &patch).map_err(|e| match e {
        UpdateError::Invalid(m) => m,
        UpdateError::Store(e) => err(e),
    })?;
    rt.apply_settings(next.clone());
    Ok(settings_json(&next))
}

fn perm_str(p: PermissionState) -> &'static str {
    match p {
        PermissionState::Granted => "granted",
        PermissionState::Denied => "denied",
        PermissionState::Unknown => "unknown",
        PermissionState::Unsupported => "unsupported",
    }
}

pub fn status_value(rt: &Arc<Runtime>) -> Value {
    let (state, reason) = rt.state();
    let settings = rt.settings();
    let tier = rt.config().map(|c| c.autonomy.level).unwrap_or_default();
    let tier_cap = policy::tier_cap(tier);
    let (recent, helper) = {
        let g = rt.lock();
        let helper = if g.sample.helper_unavailable {
            "unavailable"
        } else {
            "ready"
        };
        (g.buffer.summaries(RECENT_LIMIT), helper)
    };
    let supported = rt.sensor.platform_supported();
    let helper = if supported { helper } else { "unknown" };
    json!({
        "enabled": rt.is_enabled(),
        "state": state,
        "suspended_reason": reason,
        "paused": state == "paused",
        "paused_until": rt.paused_until(),
        "screen_capture_active": rt.screen_capture_active(),
        "platform_supported": supported,
        "lease_active": rt.lease_valid(),
        "effective_level": settings.level.min(tier_cap),
        "tier_cap": tier_cap,
        "permissions": {
            "accessibility": perm_str(rt.sensor.accessibility()),
            "screen_recording": perm_str(rt.sensor.screen_recording()),
            "helper": helper,
        },
        "recent": recent,
        "metrics": rt.metrics.snapshot(),
    })
}

pub fn status(rt: &Arc<Runtime>, config: &Config) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    Ok(status_value(rt))
}

/// Indicator lease from the shell (never from the UI).
pub fn lease(rt: &Arc<Runtime>, visible: bool, hotkey_errors: &[String]) -> Value {
    if !hotkey_errors.is_empty() {
        log::info!("[pet::companion] shell reports hotkey errors for {hotkey_errors:?}");
    }
    rt.grant_lease(visible);
    let (state, _) = rt.state();
    let settings = rt.settings();
    json!({
        "enabled": rt.is_enabled(),
        "state": state,
        "paused": state == "paused",
        "platform_supported": rt.sensor.platform_supported(),
        "screen_capture_active": rt.screen_capture_active(),
        "hotkeys": {
            "pause": settings.hotkeys.pause,
            "ask": settings.hotkeys.ask,
            "capture": settings.hotkeys.capture,
        },
    })
}

pub fn pause(
    rt: &Arc<Runtime>,
    config: &Config,
    minutes: Option<u32>,
    source: &str,
) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    if let Some(m) = minutes {
        if !(1..=1440).contains(&m) {
            return Err("invalid 'minutes': must be 1..=1440".into());
        }
    }
    rt.pause(minutes, source);
    Ok(status_value(rt))
}

pub fn resume(rt: &Arc<Runtime>, config: &Config, source: &str) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    rt.resume(source);
    Ok(status_value(rt))
}

fn gate_status(s: Suspend) -> &'static str {
    match s {
        Suspend::Off => "disabled",
        Suspend::Paused => "paused",
        Suspend::Unsupported => "unsupported",
        Suspend::NoIndicator => "no_indicator",
    }
}

fn sensor_status(e: &SensorError) -> &'static str {
    match e {
        SensorError::PermissionMissing | SensorError::PermissionRequired => "permission_required",
        SensorError::Unsupported => "unsupported",
        SensorError::HelperUnavailable(_) => "helper_unavailable",
        SensorError::SecureFieldFocused => "secure_field",
        _ => "sensor_error",
    }
}

fn outcome(status: &str, suggestion: Option<CompanionSuggestion>) -> Value {
    log::info!("[pet::companion] user request finished status={status}");
    json!({ "status": status, "suggestion": suggestion })
}

/// Blocking half of Ask: read what the user is looking at, on demand.
fn observe_for_ask(rt: &Arc<Runtime>) -> Result<ObservationEvent, &'static str> {
    let (settings, excl) = rt.settings_and_exclusions();
    let app = rt.sensor.frontmost_app().map_err(|e| sensor_status(&e))?;
    let key = app
        .bundle_id
        .as_deref()
        .unwrap_or(&app.app_name)
        .to_lowercase();
    if ALWAYS_EXCLUDED_BUNDLES.contains(&key.as_str())
        || excl
            .check_app(app.bundle_id.as_deref(), &app.app_name)
            .is_some()
    {
        rt.metrics.record_drop(DropReason::ExcludedApp);
        return Err("excluded");
    }
    if app.is_secure_field {
        rt.metrics.record_drop(DropReason::SecureField);
        return Err("secure_field");
    }
    let ctx = rt
        .sensor
        .frontmost_context(settings.sources.selection, MAX_SELECTION_CHARS)
        .map_err(|e| sensor_status(&e))?;
    let raw_title = ctx.window_title.unwrap_or_default();
    if excl.check_title(&raw_title).is_some() {
        rt.metrics.record_drop(DropReason::TitleRule);
        return Err("excluded");
    }
    if ctx.is_secure_field {
        rt.metrics.record_drop(DropReason::SecureField);
        return Err("secure_field");
    }
    if !rt.observing_allowed() {
        return Err("paused");
    }
    let title = scrub(&raw_title, &ScrubCtx::title());
    let mut text = ctx
        .selected_text
        .filter(|s| !s.trim().is_empty())
        .and_then(|s| scrub(&s, &ScrubCtx::selection(false)).into_text());
    if text.is_none() && settings.sources.clipboard {
        text = clipboard_read_event(rt, &app, Some(&title), true).and_then(|ev| ev.text);
    }
    Ok(ObservationEvent {
        at: rt.clock.utc(),
        kind: ObservationKind::Ask,
        app_name: app.app_name,
        bundle_id: app.bundle_id,
        title: title.into_text(),
        text,
        flags: ObservationFlags {
            redacted: false,
            user_initiated: true,
        },
    })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("companion task failed: {e}"))
}

pub async fn ask(rt: &Arc<Runtime>, config: &Config, source: &str) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    log::info!("[pet::companion] ask source={source}");
    if let Err(s) = rt.gate() {
        return Ok(outcome(gate_status(s), None));
    }
    let rt2 = rt.clone();
    let ev = match blocking(move || observe_for_ask(&rt2)).await? {
        Ok(ev) => ev,
        Err(status) => return Ok(outcome(status, None)),
    };
    let rt2 = rt.clone();
    let sugg = blocking(move || pipeline::process_event(&rt2, ev)).await?;
    Ok(outcome(if sugg.is_some() { "ok" } else { "paused" }, sugg))
}

pub async fn capture(rt: &Arc<Runtime>, config: &Config, source: &str) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    log::info!("[pet::companion] capture source={source}");
    if let Err(s) = rt.gate() {
        return Ok(outcome(gate_status(s), None));
    }
    if rt.sensor.screen_recording() != PermissionState::Granted {
        rt.sensor.request_screen_recording();
        return Ok(outcome("permission_required", None));
    }
    let rt2 = rt.clone();
    let res: Result<Result<ObservationEvent, &'static str>, String> = blocking(move || {
        let (settings, excl) = rt2.settings_and_exclusions();
        let app = rt2.sensor.frontmost_app().map_err(|e| sensor_status(&e))?;
        let key = app
            .bundle_id
            .as_deref()
            .unwrap_or(&app.app_name)
            .to_lowercase();
        if ALWAYS_EXCLUDED_BUNDLES.contains(&key.as_str())
            || excl
                .check_app(app.bundle_id.as_deref(), &app.app_name)
                .is_some()
        {
            rt2.metrics.record_drop(DropReason::ExcludedApp);
            return Err("excluded");
        }
        let t0 = std::time::Instant::now();
        let sample = rt2
            .sensor
            .capture_region(CAPTURE_TIMEOUT, &settings.ocr_languages)
            .map_err(|e| sensor_status(&e))?;
        Metrics::add(
            &rt2.metrics.capture_ms_total,
            t0.elapsed().as_millis() as u64,
        );
        // Paused (or the lease lapsed) while the user was dragging: discard.
        if !rt2.observing_allowed() {
            return Err("paused");
        }
        let (text, ocr_ms) = match sample {
            RegionSample::Cancelled => return Err("cancelled"),
            RegionSample::Text { text, ocr_ms } => (text, ocr_ms),
        };
        Metrics::inc(&rt2.metrics.ocr_calls);
        Metrics::add(&rt2.metrics.ocr_ms_total, ocr_ms);
        let r = scrub(&text, &ScrubCtx::ocr());
        if let Some(k) = r.drop_kind() {
            rt2.metrics.record_drop(k.drop_reason());
            return Err("dropped");
        }
        Ok(ObservationEvent {
            at: rt2.clock.utc(),
            kind: ObservationKind::Capture,
            app_name: app.app_name,
            bundle_id: app.bundle_id,
            title: None,
            text: r.into_text(),
            flags: ObservationFlags {
                redacted: false,
                user_initiated: true,
            },
        })
    })
    .await;
    let ev = match res? {
        Ok(ev) => ev,
        Err(status) => return Ok(outcome(status, None)),
    };
    let rt2 = rt.clone();
    let sugg = blocking(move || pipeline::process_event(&rt2, ev)).await?;
    Ok(outcome(if sugg.is_some() { "ok" } else { "paused" }, sugg))
}

pub fn suggestions(
    rt: &Arc<Runtime>,
    config: &Config,
    limit: Option<u64>,
    before: Option<&str>,
) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    let limit = limit.unwrap_or(50);
    if !(1..=100).contains(&limit) {
        return Err("invalid 'limit': must be 1..=100".into());
    }
    let before = before
        .map(|b| {
            chrono::DateTime::parse_from_rfc3339(b)
                .map(|d| d.with_timezone(&chrono::Utc))
                .map_err(|_| "invalid 'before': expected an RFC3339 timestamp".to_string())
        })
        .transpose()?;
    let rows = store::list_suggestions(config, limit as u32, before).map_err(err)?;
    Ok(json!(rows))
}

pub async fn suggestion_act(
    rt: &Arc<Runtime>,
    config: &Config,
    id: &str,
    action: &str,
    text: Option<&str>,
) -> Result<ActResult, String> {
    rt.bind(config).map_err(err)?;
    let action = SuggestionAction::parse(action).ok_or_else(|| {
        format!(
            "invalid 'action': expected one of {}",
            SuggestionAction::ALL
                .iter()
                .map(|a| a.as_str())
                .collect::<Vec<_>>()
                .join("|")
        )
    })?;
    actions::act(rt, config, id, action, text).await
}

pub fn data(rt: &Arc<Runtime>, config: &Config) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    let suggestions = store::list_suggestions(config, 100, None).map_err(err)?;
    let actions = store::list_actions(config, 200).map_err(err)?;
    let (s, a) = store::counts(config).map_err(err)?;
    Ok(json!({
        "suggestions": suggestions,
        "actions": actions,
        "counts": { "suggestions": s, "actions": a },
    }))
}

/// Hand-off thread ids recorded on suggestions (deduplicated). A hand-off that
/// got no thread recorded its run id instead; deleting that id is a no-op.
fn handoff_thread_ids(config: &Config) -> Result<Vec<String>, String> {
    store::with_connection(config, |c| {
        let mut stmt = c.prepare(
            "SELECT DISTINCT handoff_thread_id FROM companion_suggestions
             WHERE handoff_thread_id IS NOT NULL AND handoff_thread_id != ''",
        )?;
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(ids)
    })
    .map_err(err)
}

/// Delete the hand-off threads "Delete all" covers, through the threads domain
/// (same cleanup as the UI's thread delete). Returns how many were deleted and
/// the ids that could not be, so the user can be shown what is left.
async fn delete_handoff_threads(config: &Config, ids: Vec<String>) -> (u64, Vec<String>) {
    let mut deleted = 0;
    let mut kept = Vec::new();
    for id in ids {
        match crate::neppy::threads::ops::thread_delete_in(config.workspace_dir.clone(), id.clone())
            .await
        {
            Ok(true) => deleted += 1,
            // Already gone (or a run id recorded for a thread-less run).
            Ok(false) => {}
            Err(e) => {
                log::warn!("[pet::companion] hand-off thread delete failed: {e}");
                kept.push(id);
            }
        }
    }
    (deleted, kept)
}

pub async fn data_delete(
    rt: &Arc<Runtime>,
    config: &Config,
    suggestion_id: Option<&str>,
    all: bool,
    include_saved_notes: bool,
) -> Result<Value, String> {
    rt.bind(config).map_err(err)?;
    let mut threads = (0, Vec::new());
    let (ds, da, dn) = if all {
        rt.cancel_all_work();
        rt.lock().buffer.clear();
        // Read the hand-off threads before their rows go, then delete them:
        // "Delete all" covers the threads the companion created, not only its
        // own tables.
        let thread_ids = handoff_thread_ids(config)?;
        threads = delete_handoff_threads(config, thread_ids).await;
        let (s, a) = store::delete_all(config).map_err(err)?;
        let n = if include_saved_notes {
            actions::delete_desktop_notes(config)?
        } else {
            0
        };
        (s, a, n)
    } else if let Some(id) = suggestion_id {
        if let Some(h) = rt.lock().handoffs.remove(id) {
            h.abort();
        }
        (
            store::delete_suggestion(config, id).map_err(err)? as u64,
            0,
            0,
        )
    } else {
        return Err("invalid 'data_delete': pass 'suggestion_id' or 'all': true".into());
    };
    let (dt, kept_threads) = threads;
    log::info!(
        "[pet::companion] data deleted suggestions={ds} actions={da} notes={dn} threads={dt} \
         threads_left={}",
        kept_threads.len()
    );
    Ok(json!({
        "deleted_suggestions": ds,
        "deleted_actions": da,
        "deleted_notes": dn,
        "deleted_threads": dt,
        "undeleted_thread_ids": kept_threads,
    }))
}

pub fn request_permission(rt: &Arc<Runtime>, kind: &str) -> Result<Value, String> {
    log::info!("[pet::companion] permission requested kind={kind}");
    let (state, opened) = match kind {
        "accessibility" => (
            rt.sensor.request_accessibility(),
            rt.sensor.platform_supported(),
        ),
        "screen_recording" => {
            let s = rt.sensor.request_screen_recording();
            let open = s != PermissionState::Granted && rt.sensor.platform_supported();
            if open {
                rt.sensor.open_privacy_pane("Privacy_ScreenCapture");
            }
            (s, open)
        }
        _ => return Err("invalid 'kind': expected accessibility|screen_recording".into()),
    };
    Ok(json!({ "state": perm_str(state), "opened_settings": opened }))
}

/// Retention prune (start, then every 6 h).
pub fn prune(rt: &Arc<Runtime>) {
    let Some(config) = rt.config() else { return };
    let days = rt.settings().retention_days;
    match store::prune(&config, days, rt.clock.utc()) {
        Ok((s, a)) if s + a > 0 => {
            log::info!("[pet::companion] retention prune suggestions={s} actions={a}")
        }
        Ok(_) => {}
        Err(e) => log::warn!("[pet::companion] retention prune failed: {e:#}"),
    }
}
