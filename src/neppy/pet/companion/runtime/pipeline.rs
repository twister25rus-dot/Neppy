//! Event → suggestion. Runs synchronously on the caller (sampler thread or a
//! blocking task): buffer → activity rules → usefulness → rate limiter →
//! policy → template suggestion (no model) → store + socket. The model runs
//! only afterwards, and only for an ask / capture, or a level-2+ auto category
//! that already passed every gate ([`spawn_generation`]).
//!
//! App switches and title changes without a trigger end at the buffer: no
//! store write, no model call (PC4).

use std::sync::Arc;

use chrono::{DateTime, Utc};

use super::bus::{self, CompanionUiEvent};
use super::generate::{build_prompt, choose_provider, sanitize_output, GenProvider, GenRequest};
use super::metrics::Metrics;
use super::state::Runtime;
use crate::neppy::config::Config;
use crate::neppy::pet::companion::activity::{self, Detected};
use crate::neppy::pet::companion::policy::{self, Decision, PolicyCtx};
use crate::neppy::pet::companion::ratelimit::{parse_hhmm, RateCtx, RateVerdict};
use crate::neppy::pet::companion::store::{self, NewAction, NewSuggestion};
use crate::neppy::pet::companion::types::*;
use crate::neppy::pet::companion::usefulness::{self, UsefulnessInput};

/// Excerpt of the observed text kept on a suggestion (re-scrubbed by the store).
const CONTEXT_EXCERPT_CHARS: usize = 1500;

pub fn headline(kind: TriggerKind, app_name: &str) -> String {
    let app: String = app_name.chars().take(60).collect();
    match kind {
        TriggerKind::BuildError => format!("Build error in {app}: want an explanation?"),
        TriggerKind::EmailDraft => format!("Help with this draft in {app}?"),
        TriggerKind::Term => "Look up this term?".to_string(),
        TriggerKind::Ask => format!("You asked about {app}"),
        TriggerKind::Capture => format!("Captured text from {app}"),
    }
}

pub fn actions_for(kind: TriggerKind) -> Vec<SuggestionAction> {
    use SuggestionAction::*;
    match kind {
        TriggerKind::BuildError => vec![
            Explain,
            CopyText,
            SaveNote,
            OpenChat,
            PrepareCommand,
            Handoff,
            Dismiss,
            MuteKind,
            MuteApp,
        ],
        TriggerKind::EmailDraft => vec![
            Draft, CopyText, SaveNote, OpenChat, Handoff, Dismiss, MuteKind, MuteApp,
        ],
        TriggerKind::Term => vec![
            Explain, CopyText, SaveNote, OpenChat, Dismiss, MuteKind, MuteApp,
        ],
        TriggerKind::Ask | TriggerKind::Capture => {
            vec![
                Explain, Draft, CopyText, SaveNote, OpenChat, Handoff, Dismiss,
            ]
        }
    }
}

pub fn policy_ctx(config: &Config, initiated_by_user: bool) -> PolicyCtx {
    PolicyCtx {
        tier: config.autonomy.level,
        auto_approve_all: config.autonomy.auto_approve_all,
        initiated_by_user,
    }
}

pub fn log_action(
    config: &Config,
    suggestion_id: Option<&str>,
    category: ActionCategory,
    decision: ActionDecision,
    level: CompanionLevel,
    outcome: ActionOutcome,
    at: DateTime<Utc>,
) {
    let r = store::insert_action(
        config,
        &NewAction {
            suggestion_id: suggestion_id.map(str::to_string),
            category,
            decision,
            level,
            outcome,
            at,
        },
    );
    if let Err(e) = r {
        log::warn!("[pet::companion] action log write failed: {e:#}");
    }
}

fn quiet_hours(config: &Config) -> (chrono::NaiveTime, chrono::NaiveTime) {
    let default = (
        parse_hhmm("22:00").unwrap_or_default(),
        parse_hhmm("07:00").unwrap_or_default(),
    );
    match crate::neppy::pet::store::primary_pet(config) {
        Ok(Some(p)) => match (parse_hhmm(&p.quiet_start), parse_hhmm(&p.quiet_end)) {
            (Some(s), Some(e)) => (s, e),
            _ => default,
        },
        _ => default,
    }
}

fn app_key(ev: &ObservationEvent) -> String {
    ev.bundle_id
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_else(|| ev.app_name.trim().to_lowercase())
}

/// Process one scrubbed event. Returns the suggestion it produced, if any.
pub fn process_event(rt: &Arc<Runtime>, ev: ObservationEvent) -> Option<CompanionSuggestion> {
    // Pause wins: an event stamped at/after the pause, or processed while
    // paused, is dropped.
    if rt.is_paused() || rt.paused_at().is_some_and(|p| ev.at >= p) || !rt.is_enabled() {
        rt.metrics.record_drop(DropReason::Paused);
        return None;
    }
    Metrics::inc(&rt.metrics.events_accepted);
    let detected = activity::detect_event(&ev);
    rt.lock().buffer.push(ev.clone());
    let det = detected?;
    let config = rt.config()?;
    suggest(rt, &config, &ev, det)
}

fn suggest(
    rt: &Arc<Runtime>,
    config: &Config,
    ev: &ObservationEvent,
    det: Detected,
) -> Option<CompanionSuggestion> {
    let settings = rt.settings();
    let now = rt.clock.utc();
    let user = ev.flags.user_initiated;
    let category = det.kind.default_category();
    let key = app_key(ev);
    let level = policy::effective_level(&settings, category, config.autonomy.level);
    let decision = policy::decide(&settings, category, &policy_ctx(config, user));
    let mut score = usefulness::USER_INITIATED_SCORE;

    if !user {
        let history = store::usefulness_history(config, now).unwrap_or_default();
        let dwell = rt
            .lock()
            .sample
            .app_since
            .map(|t| rt.clock.instant().saturating_duration_since(t).as_secs())
            .unwrap_or(0);
        score = usefulness::score(
            &UsefulnessInput {
                kind: det.kind,
                bundle_id: ev.bundle_id.as_deref(),
                fingerprint: &det.fingerprint,
                dwell_secs: dwell,
                now,
            },
            &history,
            &settings,
        );
        if !usefulness::passes_without_model(score, settings.chattiness) {
            log::debug!(
                "[pet::companion] trigger below threshold kind={} score={score}",
                det.kind.as_str()
            );
            return None;
        }
        let (qs, qe) = quiet_hours(config);
        let idle = rt.lock().sample.idle_secs as u64;
        let verdict = rt.lock().ratelimit.check(
            &settings,
            &RateCtx {
                now,
                local_time: now.with_timezone(&chrono::Local).time(),
                quiet_start: qs,
                quiet_end: qe,
                idle_secs: idle,
                user_initiated: false,
                app_key: &key,
            },
        );
        if let RateVerdict::Deny(why) = verdict {
            log::debug!(
                "[pet::companion] trigger rate limited kind={} why={}",
                det.kind.as_str(),
                why.as_str()
            );
            return None;
        }
    }
    match decision {
        Decision::Drop => {
            log::debug!("[pet::companion] level 0: proactive suggestion dropped");
            return None;
        }
        Decision::Refuse => {
            log_action(
                config,
                None,
                category,
                ActionDecision::RefusedHighRisk,
                level,
                ActionOutcome::Ok,
                now,
            );
            return None;
        }
        _ => {}
    }

    let trigger = match det.kind {
        TriggerKind::Ask => SuggestionTrigger::Ask,
        TriggerKind::Capture => SuggestionTrigger::Capture,
        _ => SuggestionTrigger::Proactive,
    };
    let context = ev
        .text
        .as_ref()
        .or(ev.title.as_ref())
        .map(|t| t.excerpt(CONTEXT_EXCERPT_CHARS))
        .unwrap_or_default();
    let new = NewSuggestion {
        trigger,
        kind: det.kind,
        category,
        app_name: ev.app_name.clone(),
        bundle_id: ev.bundle_id.clone(),
        title_excerpt: ev
            .title
            .as_ref()
            .map(|t| t.excerpt(200))
            .unwrap_or_default(),
        context_excerpt: context,
        headline: headline(det.kind, &ev.app_name),
        body: None,
        score,
        actions: actions_for(det.kind),
        fingerprint: det.fingerprint.clone(),
        now,
    };
    let sugg = match store::insert_suggestion(config, &new) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[pet::companion] suggestion write failed: {e:#}");
            return None;
        }
    };
    if !user {
        rt.lock().ratelimit.record(now, &key);
    }
    Metrics::inc(&rt.metrics.suggestions_created);
    bus::publish(CompanionUiEvent::Suggestion {
        suggestion: sugg.clone(),
    });

    let auto = decision == Decision::ExecuteAuto && !user;
    if auto {
        Metrics::inc(&rt.metrics.auto_actions);
        log_action(
            config,
            Some(&sugg.id),
            category,
            ActionDecision::Auto,
            level,
            ActionOutcome::Ok,
            now,
        );
        log::info!(
            "[pet::companion] auto action category={} level={}",
            category.as_str(),
            level.as_u8()
        );
    }
    if user || auto {
        spawn_generation(rt, sugg.clone(), !user);
    }
    Some(sugg)
}

/// Generate a body for `sugg` in the background (cancelled by pause). A
/// proactive generation holds the rate limiter's single in-flight slot.
pub fn spawn_generation(rt: &Arc<Runtime>, sugg: CompanionSuggestion, proactive: bool) {
    let Some(handle) = rt.tokio() else {
        log::debug!("[pet::companion] no async runtime: generation skipped");
        return;
    };
    if proactive && !rt.lock().ratelimit.begin_generation() {
        return;
    }
    let token = rt.lock().cancel.clone();
    let rt = rt.clone();
    handle.spawn(async move {
        tokio::select! {
            _ = token.cancelled() => log::info!("[pet::companion] generation cancelled"),
            updated = generate_and_store(&rt, &sugg) => {
                if let Some(s) = updated {
                    bus::publish(CompanionUiEvent::SuggestionUpdate { suggestion: s });
                }
            }
        }
        if proactive {
            rt.lock().ratelimit.end_generation();
        }
    });
}

/// Run the generator for `sugg` (scrubbed text only) and return the body.
pub async fn generate_body(rt: &Arc<Runtime>, sugg: &CompanionSuggestion) -> Option<String> {
    let config = rt.config()?;
    let settings = rt.settings();
    let prompt = build_prompt(&GenRequest {
        kind: sugg.kind,
        category: sugg.category,
        app_name: sugg.app_name.clone(),
        title_excerpt: sugg.title_excerpt.clone(),
        excerpt: sugg.context_excerpt.clone(),
    })?;
    let provider = choose_provider(settings.allow_cloud_model, rt.generator.caps(&config));
    if provider == GenProvider::None {
        log::info!("[pet::companion] no model allowed: suggestion stays a template");
        return None;
    }
    Metrics::inc(&rt.metrics.llm_calls);
    if provider == GenProvider::Local {
        Metrics::inc(&rt.metrics.local_model_calls);
    }
    let t0 = std::time::Instant::now();
    let job = format!("pet-companion:{}", sugg.id);
    let out = rt.generator.run(&config, provider, &job, &prompt).await;
    log::info!(
        "[pet::companion] generation done id={} provider={} ok={} ms={}",
        sugg.id,
        provider.as_str(),
        out.is_ok(),
        t0.elapsed().as_millis()
    );
    sanitize_output(&out.ok()?)
}

async fn generate_and_store(
    rt: &Arc<Runtime>,
    sugg: &CompanionSuggestion,
) -> Option<CompanionSuggestion> {
    let body = generate_body(rt, sugg).await?;
    if rt.is_paused() {
        return None;
    }
    let config = rt.config()?;
    store::set_body(&config, &sugg.id, &body).ok()?;
    store::get_suggestion(&config, &sugg.id).ok().flatten()
}
