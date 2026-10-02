//! Rate limiter for PROACTIVE surfacing. Pure, with an injected clock and
//! injected local time. User-initiated Ask / Capture bypass every rule except
//! the in-flight generation guard.
//!
//! Rules: global minimum interval, hourly cap, per-app cooldown, the pet's
//! quiet hours (reused from the research surfacer), no proactive surfacing when
//! the user has been idle for more than 5 minutes, and at most one in-flight
//! generation.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveTime, Utc};

use super::settings::CompanionSettings;
use crate::neppy::pet::surfacer::in_quiet_hours;

pub const IDLE_LIMIT_SECS: u64 = 300;

/// Why a surfacing was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateDeny {
    InFlight,
    QuietHours,
    Idle,
    MinInterval,
    HourlyCap,
    AppCooldown,
}

impl RateDeny {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InFlight => "in_flight",
            Self::QuietHours => "quiet_hours",
            Self::Idle => "idle",
            Self::MinInterval => "min_interval",
            Self::HourlyCap => "hourly_cap",
            Self::AppCooldown => "app_cooldown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateVerdict {
    Allow,
    Deny(RateDeny),
}

/// Per-check context.
#[derive(Debug, Clone, Copy)]
pub struct RateCtx<'a> {
    pub now: DateTime<Utc>,
    /// Local wall-clock time, for quiet hours.
    pub local_time: NaiveTime,
    pub quiet_start: NaiveTime,
    pub quiet_end: NaiveTime,
    pub idle_secs: u64,
    pub user_initiated: bool,
    /// Lowercased bundle id (or app name) of the frontmost app.
    pub app_key: &'a str,
}

/// Parse `HH:MM` (as stored in the pet row).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").ok()
}

#[derive(Debug, Clone, Default)]
pub struct RateLimiter {
    last_surfaced: Option<DateTime<Utc>>,
    surfaced: Vec<DateTime<Utc>>,
    per_app: HashMap<String, DateTime<Utc>>,
    in_flight: u32,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild from stored proactive suggestions `(created_at, app_key)`, so a
    /// restart does not reset the budget.
    pub fn from_history(history: &[(DateTime<Utc>, String)]) -> Self {
        let mut rl = Self::new();
        for (at, app) in history {
            rl.record(*at, app);
        }
        rl
    }

    pub fn check(&self, settings: &CompanionSettings, ctx: &RateCtx<'_>) -> RateVerdict {
        if self.in_flight >= 1 {
            return RateVerdict::Deny(RateDeny::InFlight);
        }
        if ctx.user_initiated {
            return RateVerdict::Allow;
        }
        if in_quiet_hours(ctx.local_time, ctx.quiet_start, ctx.quiet_end) {
            return RateVerdict::Deny(RateDeny::QuietHours);
        }
        if ctx.idle_secs > IDLE_LIMIT_SECS {
            return RateVerdict::Deny(RateDeny::Idle);
        }
        if let Some(last) = self.last_surfaced {
            if ctx.now - last < Duration::minutes(settings.min_interval_min as i64) {
                return RateVerdict::Deny(RateDeny::MinInterval);
            }
        }
        let hour_ago = ctx.now - Duration::hours(1);
        let in_hour = self.surfaced.iter().filter(|t| **t > hour_ago).count();
        if in_hour >= settings.max_per_hour as usize {
            return RateVerdict::Deny(RateDeny::HourlyCap);
        }
        if let Some(last) = self.per_app.get(ctx.app_key) {
            if ctx.now - *last < Duration::minutes(settings.app_cooldown_min as i64) {
                return RateVerdict::Deny(RateDeny::AppCooldown);
            }
        }
        RateVerdict::Allow
    }

    /// Record a surfaced proactive suggestion.
    pub fn record(&mut self, now: DateTime<Utc>, app_key: &str) {
        self.last_surfaced = Some(self.last_surfaced.map_or(now, |l| l.max(now)));
        self.surfaced.push(now);
        let cutoff = now - Duration::hours(1);
        self.surfaced.retain(|t| *t > cutoff);
        let e = self.per_app.entry(app_key.to_lowercase()).or_insert(now);
        if *e < now {
            *e = now;
        }
    }

    pub fn begin_generation(&mut self) -> bool {
        if self.in_flight >= 1 {
            return false;
        }
        self.in_flight += 1;
        true
    }

    pub fn end_generation(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }
}

#[cfg(test)]
#[path = "ratelimit_tests.rs"]
mod ratelimit_tests;
