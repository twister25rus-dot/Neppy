//! Usefulness score for a detected trigger. Pure.
//!
//! `score = base(kind) + dwell_bonus(<=10) - dismiss_penalty - novelty - mute`
//!
//! * base: BuildError 70, EmailDraft 55, Term 60.
//! * novelty: -100 if the fingerprint was surfaced in the last 30 minutes.
//! * dismiss penalty: -20 per dismissal of the same kind + app in 24 hours.
//! * mute: -1000 (muted kind or muted app).
//! * Ask / Capture are user initiated: always [`USER_INITIATED_SCORE`], none of
//!   the penalties apply (re-asking about the same thing must work).
//!
//! Thresholds by chattiness: quiet 75, normal 60, eager 45. The gray zone is
//! `threshold +- 10`; inside it an optional on-device yes/no may decide
//! (timeout or failure counts as "no", and it is never the cloud model).

use chrono::{DateTime, Duration, Utc};

use super::settings::CompanionSettings;
use super::types::{Chattiness, TriggerKind};

pub const USER_INITIATED_SCORE: i32 = 100;
pub const NOVELTY_WINDOW_MIN: i64 = 30;
pub const NOVELTY_PENALTY: i32 = 100;
pub const DISMISS_WINDOW_HOURS: i64 = 24;
pub const DISMISS_PENALTY: i32 = 20;
pub const MUTE_PENALTY: i32 = 1000;
pub const GRAY_BAND: i32 = 10;
pub const MAX_DWELL_BONUS: i32 = 10;

/// What the store knows about recent suggestions.
#[derive(Debug, Clone, Default)]
pub struct UsefulnessHistory {
    /// `(fingerprint, surfaced_at)` of recent suggestions.
    pub seen: Vec<(String, DateTime<Utc>)>,
    /// `(kind, lowercased bundle id, dismissed/created at)`.
    pub dismissals: Vec<(TriggerKind, String, DateTime<Utc>)>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsefulnessInput<'a> {
    pub kind: TriggerKind,
    pub bundle_id: Option<&'a str>,
    pub fingerprint: &'a str,
    /// Seconds the user has stayed on this context.
    pub dwell_secs: u64,
    pub now: DateTime<Utc>,
}

pub fn base_score(kind: TriggerKind) -> i32 {
    match kind {
        TriggerKind::BuildError => 70,
        TriggerKind::EmailDraft => 55,
        TriggerKind::Term => 60,
        TriggerKind::Ask | TriggerKind::Capture => USER_INITIATED_SCORE,
    }
}

/// Up to +10: one point per 3 seconds of dwell.
pub fn dwell_bonus(dwell_secs: u64) -> i32 {
    ((dwell_secs / 3).min(MAX_DWELL_BONUS as u64)) as i32
}

pub fn is_muted(settings: &CompanionSettings, kind: TriggerKind, bundle_id: Option<&str>) -> bool {
    settings.muted_kinds.contains(&kind)
        || bundle_id.is_some_and(|b| {
            settings
                .muted_apps
                .iter()
                .any(|m| m.eq_ignore_ascii_case(b))
        })
}

pub fn score(
    input: &UsefulnessInput<'_>,
    history: &UsefulnessHistory,
    settings: &CompanionSettings,
) -> i32 {
    if !input.kind.is_proactive() {
        return USER_INITIATED_SCORE;
    }
    let mut s = base_score(input.kind) + dwell_bonus(input.dwell_secs);
    let novelty_cutoff = input.now - Duration::minutes(NOVELTY_WINDOW_MIN);
    if history
        .seen
        .iter()
        .any(|(fp, at)| fp == input.fingerprint && *at >= novelty_cutoff && *at <= input.now)
    {
        s -= NOVELTY_PENALTY;
    }
    let dismiss_cutoff = input.now - Duration::hours(DISMISS_WINDOW_HOURS);
    let bundle = input.bundle_id.unwrap_or("").to_lowercase();
    let dismissed = history
        .dismissals
        .iter()
        .filter(|(k, b, at)| *k == input.kind && *b == bundle && *at >= dismiss_cutoff)
        .count() as i32;
    s -= DISMISS_PENALTY * dismissed;
    if is_muted(settings, input.kind, input.bundle_id) {
        s -= MUTE_PENALTY;
    }
    s
}

pub fn threshold(chattiness: Chattiness) -> i32 {
    match chattiness {
        Chattiness::Quiet => 75,
        Chattiness::Normal => 60,
        Chattiness::Eager => 45,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// At or above `threshold + 10`: surface.
    Accept,
    /// Within `threshold +- 10`: the optional local model decides.
    Gray,
    /// Below `threshold - 10`: do not surface.
    Reject,
}

pub fn verdict(score: i32, chattiness: Chattiness) -> Verdict {
    let t = threshold(chattiness);
    if score >= t + GRAY_BAND {
        Verdict::Accept
    } else if score >= t - GRAY_BAND {
        Verdict::Gray
    } else {
        Verdict::Reject
    }
}

/// Decision without a local model: the plain threshold.
pub fn passes_without_model(score: i32, chattiness: Chattiness) -> bool {
    score >= threshold(chattiness)
}

#[cfg(test)]
#[path = "usefulness_tests.rs"]
mod usefulness_tests;
