use chrono::{Duration, TimeZone, Utc};

use super::*;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 2, 12, 0, 0).unwrap()
}

fn input(kind: TriggerKind, dwell: u64) -> UsefulnessInput<'static> {
    UsefulnessInput {
        kind,
        bundle_id: Some("com.apple.Terminal"),
        fingerprint: "fp1",
        dwell_secs: dwell,
        now: now(),
    }
}

fn s() -> CompanionSettings {
    CompanionSettings::default()
}

#[test]
fn base_scores_and_dwell_bonus() {
    let h = UsefulnessHistory::default();
    assert_eq!(score(&input(TriggerKind::BuildError, 0), &h, &s()), 70);
    assert_eq!(score(&input(TriggerKind::EmailDraft, 0), &h, &s()), 55);
    assert_eq!(score(&input(TriggerKind::Term, 0), &h, &s()), 60);
    assert_eq!(score(&input(TriggerKind::Term, 9), &h, &s()), 63);
    assert_eq!(score(&input(TriggerKind::Term, 10_000), &h, &s()), 70);
    assert_eq!(dwell_bonus(0), 0);
    assert_eq!(dwell_bonus(30), 10);
}

#[test]
fn thresholds_by_chattiness_and_gray_zone() {
    assert_eq!(threshold(Chattiness::Quiet), 75);
    assert_eq!(threshold(Chattiness::Normal), 60);
    assert_eq!(threshold(Chattiness::Eager), 45);
    assert_eq!(verdict(70, Chattiness::Normal), Verdict::Accept);
    assert_eq!(verdict(69, Chattiness::Normal), Verdict::Gray);
    assert_eq!(verdict(50, Chattiness::Normal), Verdict::Gray);
    assert_eq!(verdict(49, Chattiness::Normal), Verdict::Reject);
    assert!(passes_without_model(60, Chattiness::Normal));
    assert!(!passes_without_model(59, Chattiness::Normal));
    // Email draft (55) clears eager and is gray on normal, rejected on quiet.
    assert_eq!(verdict(55, Chattiness::Eager), Verdict::Accept);
    assert_eq!(verdict(55, Chattiness::Normal), Verdict::Gray);
    assert_eq!(verdict(55, Chattiness::Quiet), Verdict::Reject);
}

#[test]
fn novelty_penalises_recent_fingerprints_only() {
    let mut h = UsefulnessHistory::default();
    h.seen.push(("fp1".into(), now() - Duration::minutes(10)));
    assert_eq!(score(&input(TriggerKind::BuildError, 0), &h, &s()), -30);
    let mut old = UsefulnessHistory::default();
    old.seen.push(("fp1".into(), now() - Duration::minutes(31)));
    assert_eq!(score(&input(TriggerKind::BuildError, 0), &old, &s()), 70);
    let mut other = UsefulnessHistory::default();
    other
        .seen
        .push(("fp2".into(), now() - Duration::minutes(1)));
    assert_eq!(score(&input(TriggerKind::BuildError, 0), &other, &s()), 70);
}

#[test]
fn dismissals_of_the_same_kind_and_app_in_24h_cost_20_each() {
    let mut h = UsefulnessHistory::default();
    let b = "com.apple.terminal".to_string();
    h.dismissals.push((
        TriggerKind::BuildError,
        b.clone(),
        now() - Duration::hours(1),
    ));
    h.dismissals.push((
        TriggerKind::BuildError,
        b.clone(),
        now() - Duration::hours(23),
    ));
    h.dismissals.push((
        TriggerKind::BuildError,
        b.clone(),
        now() - Duration::hours(25),
    ));
    h.dismissals
        .push((TriggerKind::Term, b.clone(), now() - Duration::hours(1)));
    h.dismissals.push((
        TriggerKind::BuildError,
        "com.other".into(),
        now() - Duration::hours(1),
    ));
    assert_eq!(score(&input(TriggerKind::BuildError, 0), &h, &s()), 30);
}

#[test]
fn mute_kills_the_score() {
    let h = UsefulnessHistory::default();
    let mut kind_muted = s();
    kind_muted.muted_kinds.push(TriggerKind::BuildError);
    assert!(score(&input(TriggerKind::BuildError, 30), &h, &kind_muted) < -900);
    let mut app_muted = s();
    app_muted.muted_apps.push("COM.APPLE.TERMINAL".into());
    assert!(score(&input(TriggerKind::BuildError, 30), &h, &app_muted) < -900);
    assert!(is_muted(
        &app_muted,
        TriggerKind::Term,
        Some("com.apple.terminal")
    ));
    assert!(!is_muted(
        &app_muted,
        TriggerKind::Term,
        Some("com.apple.notes")
    ));
}

#[test]
fn user_initiated_kinds_ignore_penalties() {
    let mut h = UsefulnessHistory::default();
    h.seen.push(("fp1".into(), now()));
    let mut muted = s();
    muted.muted_kinds.push(TriggerKind::BuildError);
    for k in [TriggerKind::Ask, TriggerKind::Capture] {
        assert_eq!(score(&input(k, 0), &h, &muted), USER_INITIATED_SCORE);
        assert_eq!(
            verdict(USER_INITIATED_SCORE, Chattiness::Quiet),
            Verdict::Accept
        );
    }
}
