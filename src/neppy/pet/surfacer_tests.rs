use std::collections::HashSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveTime, TimeZone, Utc};

use super::surfacer::*;
use super::types::*;

pub(super) fn note(id: &str, kind: PetNoteKind, urgency: u8) -> PetNote {
    PetNote {
        id: id.into(),
        pet_id: "pet".into(),
        source: PetNoteSource::Email,
        kind,
        title: format!("title {id}"),
        body: String::new(),
        urgency,
        due_at: None,
        goal_ids: vec![],
        proposed_action: None,
        fingerprint: format!("fp-{id}"),
        injection_flagged: false,
        score: None,
        bucket: None,
        state: PetNoteState::New,
        digest_id: None,
        created_at: base_now(),
        surfaced_at: None,
        notified_at: None,
    }
}

fn base_now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap()
}

fn t(h: u32, m: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(h, m, 0).unwrap()
}

fn goal(id: &str, text: &str) -> PetGoal {
    PetGoal {
        id: id.into(),
        text: text.into(),
        created_at: base_now(),
    }
}

struct Fixture {
    goals: Vec<PetGoal>,
    seen: HashSet<String>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            goals: vec![goal("g1", "Finish the PGCE portfolio")],
            seen: HashSet::new(),
        }
    }

    fn ctx(&self, notified_today: u32, budget: u32) -> RankCtx<'_, Utc> {
        RankCtx {
            now: base_now(),
            tz: &Utc,
            goals: &self.goals,
            seen_fingerprints: &self.seen,
            notified_today,
            budget,
            quiet: (t(0, 0), t(0, 0)),
        }
    }
}

#[test]
fn worked_cases_score_exactly() {
    let f = Fixture::new();
    let ctx = f.ctx(0, 3);
    let now = base_now();
    let due = |h: i64| Some(now + Duration::hours(h));
    let mut cases = Vec::new();

    let mut a = note("a", PetNoteKind::Request, 3);
    a.due_at = due(2);
    a.goal_ids = vec!["g1".into()];
    cases.push((a, 95, Bucket::Notify));

    let mut b = note("b", PetNoteKind::Deadline, 3);
    b.due_at = due(20);
    cases.push((b, 71, Bucket::Notify));

    let mut c = note("c", PetNoteKind::Deadline, 2);
    c.due_at = due(48);
    cases.push((c, 53, Bucket::Digest));

    cases.push((note("d", PetNoteKind::Fyi, 1), 12, Bucket::Drop));

    let mut e = note("e", PetNoteKind::Meeting, 1);
    e.due_at = due(30);
    e.goal_ids = vec!["g1".into()];
    cases.push((e, 59, Bucket::Digest));

    for (n, want, bucket) in cases {
        assert_eq!(score(&n, &ctx), want, "score for {}", n.id);
        assert_eq!(bucket_for_score(want), bucket, "bucket for {}", n.id);
    }
}

#[test]
fn thresholds_are_inclusive_at_25_and_70() {
    assert_eq!(bucket_for_score(24), Bucket::Drop);
    assert_eq!(bucket_for_score(25), Bucket::Digest);
    assert_eq!(bucket_for_score(69), Bucket::Digest);
    assert_eq!(bucket_for_score(70), Bucket::Notify);
    assert_eq!(bucket_for_score(100), Bucket::Notify);
}

#[test]
fn goal_relevance_by_tokens() {
    let goals = vec![goal("g1", "Finish the PGCE portfolio")];
    let mut n = note("n", PetNoteKind::Fyi, 0);
    n.title = "Mentor asks for PGCE portfolio draft".into();
    assert_eq!(goal_relevance(&n, &goals), 100);
    n.title = "PGCE timetable".into();
    assert_eq!(goal_relevance(&n, &goals), 50);
    n.title = "Newsletter arrived".into();
    assert_eq!(goal_relevance(&n, &goals), 0);
    // Stopwords and short tokens never count.
    assert!(tokens("this that from the pgce").contains("pgce"));
    assert!(!tokens("this that from the pgce").contains("this"));
}

#[test]
fn overdue_and_far_future_time_pressure() {
    let f = Fixture::new();
    let ctx = f.ctx(0, 3);
    let mut overdue = note("o", PetNoteKind::Fyi, 0);
    overdue.due_at = Some(base_now() - Duration::hours(72));
    // T=.3 → 10.5, K=.2 → 5 → 15.5 → 16 (half rounds up)
    assert_eq!(score(&overdue, &ctx), 16);
    let mut far = note("f", PetNoteKind::Fyi, 0);
    far.due_at = Some(base_now() + Duration::days(30));
    assert_eq!(score(&far, &ctx), 5);
}

#[test]
fn quiet_hours_wrap_midnight_and_equal_disables() {
    assert!(in_quiet_hours(t(23, 0), t(22, 0), t(7, 0)));
    assert!(in_quiet_hours(t(3, 0), t(22, 0), t(7, 0)));
    assert!(!in_quiet_hours(t(7, 0), t(22, 0), t(7, 0)));
    assert!(in_quiet_hours(t(22, 0), t(22, 0), t(7, 0)));
    assert!(!in_quiet_hours(t(12, 0), t(22, 0), t(7, 0)));
    assert!(in_quiet_hours(t(13, 0), t(12, 0), t(14, 0)));
    assert!(!in_quiet_hours(t(14, 0), t(12, 0), t(14, 0)));
    assert!(!in_quiet_hours(t(3, 0), t(9, 0), t(9, 0)));
}

fn urgent(id: &str, minutes_after: i64) -> PetNote {
    let mut n = note(id, PetNoteKind::Request, 3);
    n.due_at = Some(base_now() + Duration::hours(2));
    n.created_at = base_now() - Duration::minutes(60 - minutes_after);
    n
}

#[test]
fn budget_allocation_is_deterministic_under_ties() {
    let f = Fixture::new();
    // Three equal-score candidates, budget 2 with 1 already used today → one slot.
    let notes = vec![urgent("c", 2), urgent("a", 1), urgent("b", 1)];
    let ranked = rank(&notes, &f.ctx(1, 2));
    let notified: Vec<&str> = ranked
        .iter()
        .filter(|s| s.bucket == Bucket::Notify)
        .map(|s| s.note_id.as_str())
        .collect();
    // Earliest created_at wins; `a` beats `b` on id.
    assert_eq!(notified, vec!["a"]);
    for s in &ranked {
        if s.note_id != "a" {
            assert_eq!(s.bucket, Bucket::Digest);
            assert_eq!(s.state, PetNoteState::Queued);
        } else {
            assert_eq!(s.state, PetNoteState::Notified);
        }
    }
}

#[test]
fn injection_flagged_and_quiet_hours_never_notify() {
    let mut f = Fixture::new();
    let mut flagged = urgent("x", 0);
    flagged.injection_flagged = true;
    let ranked = rank(&[flagged], &f.ctx(0, 10));
    assert_eq!(ranked[0].bucket, Bucket::Digest);

    f.goals.clear();
    let mut ctx = f.ctx(0, 10);
    ctx.quiet = (t(11, 0), t(13, 0));
    let ranked = rank(&[urgent("y", 0)], &ctx);
    assert_eq!(ranked[0].bucket, Bucket::Digest);
}

#[test]
fn duplicates_within_seven_days_and_within_batch_are_dropped() {
    let mut f = Fixture::new();
    f.seen.insert("fp-seen".into());
    let mut seen = urgent("s", 0);
    seen.fingerprint = "fp-seen".into();
    let mut first = urgent("d1", 1);
    first.fingerprint = "fp-same".into();
    let mut second = urgent("d2", 2);
    second.fingerprint = "fp-same".into();
    let ranked = rank(&[second, seen, first], &f.ctx(0, 10));
    let by_id = |id: &str| ranked.iter().find(|s| s.note_id == id).unwrap().clone();
    assert_eq!(by_id("s").bucket, Bucket::Duplicate);
    assert_eq!(by_id("s").state, PetNoteState::Dropped);
    assert_eq!(by_id("d1").bucket, Bucket::Notify);
    assert_eq!(by_id("d2").bucket, Bucket::Duplicate);
    // The 7-day window itself is applied by the store query; an older
    // fingerprint is simply absent from `seen_fingerprints`.
    let fresh = Fixture::new();
    let mut again = urgent("s2", 0);
    again.fingerprint = "fp-seen".into();
    assert_eq!(rank(&[again], &fresh.ctx(0, 10))[0].bucket, Bucket::Notify);
}

#[test]
fn next_local_occurrence_is_strictly_after_now() {
    let tz = FixedOffset::east_opt(2 * 3600).unwrap();
    let seven = t(7, 0);
    // 06:00 local (04:00Z) → today 07:00 local (05:00Z).
    let before = Utc.with_ymd_and_hms(2026, 9, 30, 4, 0, 0).unwrap();
    assert_eq!(
        next_local_occurrence(before, seven, &tz),
        Utc.with_ymd_and_hms(2026, 9, 30, 5, 0, 0).unwrap()
    );
    // Exactly 07:00 local → tomorrow.
    let equal = Utc.with_ymd_and_hms(2026, 9, 30, 5, 0, 0).unwrap();
    assert_eq!(
        next_local_occurrence(equal, seven, &tz),
        Utc.with_ymd_and_hms(2026, 10, 1, 5, 0, 0).unwrap()
    );
    // After → tomorrow.
    let after = Utc.with_ymd_and_hms(2026, 9, 30, 10, 0, 0).unwrap();
    assert_eq!(
        next_local_occurrence(after, seven, &tz),
        Utc.with_ymd_and_hms(2026, 10, 1, 5, 0, 0).unwrap()
    );
    // Waking three days after a missed slot yields exactly the next one.
    let woke = Utc.with_ymd_and_hms(2026, 10, 3, 9, 30, 0).unwrap();
    assert_eq!(
        next_local_occurrence(woke, seven, &tz),
        Utc.with_ymd_and_hms(2026, 10, 4, 5, 0, 0).unwrap()
    );
}

#[test]
fn next_local_occurrence_skips_a_dst_gap() {
    // Europe/London springs forward at 01:00 → 02:00 on 2026-03-29.
    let tz = chrono_tz::Europe::London;
    let now = Utc.with_ymd_and_hms(2026, 3, 28, 12, 0, 0).unwrap();
    let got = next_local_occurrence(now, t(1, 30), &tz);
    // 01:30 does not exist that day; the next valid minute is 02:00 BST = 01:00Z.
    assert_eq!(got, Utc.with_ymd_and_hms(2026, 3, 29, 1, 0, 0).unwrap());
}
