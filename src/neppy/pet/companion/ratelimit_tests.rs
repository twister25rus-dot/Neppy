use chrono::{Duration, NaiveTime, TimeZone, Utc};

use super::*;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 2, 12, 0, 0).unwrap()
}

fn hm(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn ctx(now: DateTime<Utc>, app: &'static str) -> RateCtx<'static> {
    RateCtx {
        now,
        local_time: hm(12, 0),
        quiet_start: hm(22, 0),
        quiet_end: hm(7, 0),
        idle_secs: 0,
        user_initiated: false,
        app_key: app,
    }
}

fn s() -> CompanionSettings {
    CompanionSettings::default()
}

#[test]
fn fresh_limiter_allows() {
    assert_eq!(
        RateLimiter::new().check(&s(), &ctx(t0(), "a")),
        RateVerdict::Allow
    );
}

#[test]
fn min_interval_then_app_cooldown() {
    let mut rl = RateLimiter::new();
    rl.record(t0(), "a");
    let at = |m: i64, app| ctx(t0() + Duration::minutes(m), app);
    assert_eq!(
        rl.check(&s(), &at(5, "b")),
        RateVerdict::Deny(RateDeny::MinInterval)
    );
    assert_eq!(rl.check(&s(), &at(11, "b")), RateVerdict::Allow);
    assert_eq!(
        rl.check(&s(), &at(11, "a")),
        RateVerdict::Deny(RateDeny::AppCooldown)
    );
    assert_eq!(rl.check(&s(), &at(31, "a")), RateVerdict::Allow);
}

#[test]
fn hourly_cap_and_window_slide() {
    let mut cfg = s();
    cfg.min_interval_min = 1;
    cfg.app_cooldown_min = 0;
    cfg.max_per_hour = 3;
    let mut rl = RateLimiter::new();
    for m in [0, 2, 4] {
        rl.record(t0() + Duration::minutes(m), "a");
    }
    assert_eq!(
        rl.check(&cfg, &ctx(t0() + Duration::minutes(6), "a")),
        RateVerdict::Deny(RateDeny::HourlyCap)
    );
    // At minute 61 the first one has aged out.
    assert_eq!(
        rl.check(&cfg, &ctx(t0() + Duration::minutes(61), "a")),
        RateVerdict::Allow
    );
}

#[test]
fn quiet_hours_across_midnight() {
    let rl = RateLimiter::new();
    let at = |h, m| RateCtx {
        local_time: hm(h, m),
        ..ctx(t0(), "a")
    };
    assert_eq!(
        rl.check(&s(), &at(23, 30)),
        RateVerdict::Deny(RateDeny::QuietHours)
    );
    assert_eq!(
        rl.check(&s(), &at(3, 0)),
        RateVerdict::Deny(RateDeny::QuietHours)
    );
    assert_eq!(
        rl.check(&s(), &at(6, 59)),
        RateVerdict::Deny(RateDeny::QuietHours)
    );
    assert_eq!(rl.check(&s(), &at(7, 0)), RateVerdict::Allow);
    assert_eq!(rl.check(&s(), &at(12, 0)), RateVerdict::Allow);
    let same = RateCtx {
        quiet_start: hm(9, 0),
        quiet_end: hm(9, 0),
        ..at(9, 0)
    };
    assert_eq!(rl.check(&s(), &same), RateVerdict::Allow);
}

#[test]
fn idle_users_get_no_proactive_suggestions() {
    let rl = RateLimiter::new();
    let idle = |secs| RateCtx {
        idle_secs: secs,
        ..ctx(t0(), "a")
    };
    assert_eq!(rl.check(&s(), &idle(300)), RateVerdict::Allow);
    assert_eq!(
        rl.check(&s(), &idle(301)),
        RateVerdict::Deny(RateDeny::Idle)
    );
}

#[test]
fn user_initiated_bypasses_everything_except_in_flight() {
    let mut rl = RateLimiter::new();
    rl.record(t0(), "a");
    let user = RateCtx {
        user_initiated: true,
        idle_secs: 10_000,
        local_time: hm(23, 0),
        ..ctx(t0(), "a")
    };
    assert_eq!(rl.check(&s(), &user), RateVerdict::Allow);
    assert!(rl.begin_generation());
    assert_eq!(rl.check(&s(), &user), RateVerdict::Deny(RateDeny::InFlight));
    rl.end_generation();
    assert_eq!(rl.check(&s(), &user), RateVerdict::Allow);
}

#[test]
fn at_most_one_generation_in_flight() {
    let mut rl = RateLimiter::new();
    assert!(rl.begin_generation());
    assert!(!rl.begin_generation());
    assert_eq!(rl.in_flight(), 1);
    assert_eq!(
        rl.check(&s(), &ctx(t0(), "a")),
        RateVerdict::Deny(RateDeny::InFlight)
    );
    rl.end_generation();
    rl.end_generation();
    assert_eq!(rl.in_flight(), 0);
    assert!(rl.begin_generation());
}

#[test]
fn from_history_seeds_the_budget() {
    let hist = vec![(t0() - Duration::minutes(3), "A".to_string())];
    let rl = RateLimiter::from_history(&hist);
    assert_eq!(
        rl.check(&s(), &ctx(t0(), "b")),
        RateVerdict::Deny(RateDeny::MinInterval)
    );
    assert_eq!(parse_hhmm("22:00"), Some(hm(22, 0)));
    assert_eq!(parse_hhmm("nope"), None);
}
