//! Deterministic ranking of Pet notes — no LLM. The research lane only
//! classifies each finding (`kind`, `urgency`); scoring, bucketing, the daily
//! interrupt budget and quiet hours are pure functions of stored data here.
//!
//! ```text
//! T (time pressure, h = hours(due_at - now)): none → 0
//!    h < -48 → .3 ; -48 ≤ h ≤ 4 → 1 ; ≤ 24 → .8 ; ≤ 72 → .5 ; ≤ 168 → .2 ; else 0
//! K (kind): deadline .9, request .8, meeting .6, change .5, proposal .5, fyi .2, idea .2
//! U (urgency): urgency / 3
//! G (goal relevance): 1 on a goal-id match, else max over goals of
//!    min(1, |tokens(title body) ∩ tokens(goal)| / 2)
//! score = round(100 · (.35T + .25K + .20U + .20G))
//! ```
//! Computed in integer hundredths so the documented worked cases (95/71/53/12/59)
//! are exact rather than at the mercy of float rounding.

use std::collections::HashSet;

use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};

use super::types::{Bucket, PetGoal, PetNote, PetNoteKind, PetNoteState};

/// Scores at or above this interrupt the user (subject to budget / quiet hours).
pub(crate) const NOTIFY_THRESHOLD: u8 = 70;
/// Scores at or above this (and below notify) go to the digest; below are dropped.
pub(crate) const DIGEST_THRESHOLD: u8 = 25;

/// Tokens this common never count towards goal relevance.
const STOPWORDS: &[&str] = &[
    "about", "above", "after", "again", "also", "been", "before", "being", "below", "between",
    "both", "could", "does", "doing", "down", "during", "each", "from", "further", "have",
    "having", "here", "into", "just", "more", "most", "only", "other", "over", "same", "some",
    "such", "than", "that", "their", "them", "then", "there", "these", "this",
];

/// Inputs to one ranking pass.
pub(crate) struct RankCtx<'a, Tz: TimeZone> {
    pub now: DateTime<Utc>,
    pub tz: &'a Tz,
    pub goals: &'a [PetGoal],
    /// Fingerprints of notes already surfaced in the last 7 days.
    pub seen_fingerprints: &'a HashSet<String>,
    pub notified_today: u32,
    pub budget: u32,
    /// `(quiet_start, quiet_end)`, device-local; equal values disable quiet hours.
    pub quiet: (NaiveTime, NaiveTime),
}

/// The surfacer's verdict for one note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SurfacedNote {
    pub note_id: String,
    pub score: u8,
    pub bucket: Bucket,
    pub state: PetNoteState,
}

/// Lowercase alphanumeric tokens of at least 4 chars, minus stopwords.
pub(crate) fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 4 && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect()
}

/// Time pressure in hundredths.
fn time_pressure(note: &PetNote, now: DateTime<Utc>) -> u32 {
    let Some(due) = note.due_at else { return 0 };
    let minutes = (due - now).num_minutes();
    // Compare in minutes to avoid float edges: 4h = 240, 24h = 1440, …
    match minutes {
        m if m < -48 * 60 => 30,
        m if m <= 4 * 60 => 100,
        m if m <= 24 * 60 => 80,
        m if m <= 72 * 60 => 50,
        m if m <= 168 * 60 => 20,
        _ => 0,
    }
}

fn kind_weight(kind: PetNoteKind) -> u32 {
    match kind {
        PetNoteKind::Deadline => 90,
        PetNoteKind::Request => 80,
        PetNoteKind::Meeting => 60,
        PetNoteKind::Change | PetNoteKind::Proposal => 50,
        PetNoteKind::Fyi | PetNoteKind::Idea => 20,
    }
}

/// Goal relevance in hundredths (always 0, 50 or 100).
pub(crate) fn goal_relevance(note: &PetNote, goals: &[PetGoal]) -> u32 {
    if note
        .goal_ids
        .iter()
        .any(|gid| goals.iter().any(|g| &g.id == gid))
    {
        return 100;
    }
    let note_tokens = tokens(&format!("{} {}", note.title, note.body));
    goals
        .iter()
        .map(|g| {
            let overlap = tokens(&g.text).intersection(&note_tokens).count() as u32;
            (overlap * 50).min(100)
        })
        .max()
        .unwrap_or(0)
}

/// Score in `[0, 100]`.
pub(crate) fn score<Tz: TimeZone>(note: &PetNote, ctx: &RankCtx<'_, Tz>) -> u8 {
    let t = time_pressure(note, ctx.now);
    let k = kind_weight(note.kind);
    let g = goal_relevance(note, ctx.goals);
    let u = u32::from(note.urgency.min(3));
    // 100·score = 35T + 25K + 20·(100u/3) + 20G, all in hundredths; ×3 to stay integral.
    let num = 3 * (35 * t + 25 * k + 20 * g) + 2000 * u;
    ((num + 150) / 300).min(100) as u8
}

/// Base bucket for a score (before budget / quiet-hours demotion).
pub(crate) fn bucket_for_score(score: u8) -> Bucket {
    if score >= NOTIFY_THRESHOLD {
        Bucket::Notify
    } else if score >= DIGEST_THRESHOLD {
        Bucket::Digest
    } else {
        Bucket::Drop
    }
}

/// `[start, end)` membership that wraps midnight; `start == end` disables.
pub(crate) fn in_quiet_hours(local: NaiveTime, start: NaiveTime, end: NaiveTime) -> bool {
    if start == end {
        false
    } else if start < end {
        local >= start && local < end
    } else {
        local >= start || local < end
    }
}

fn state_for(bucket: Bucket) -> PetNoteState {
    match bucket {
        Bucket::Notify => PetNoteState::Notified,
        Bucket::Digest => PetNoteState::Queued,
        Bucket::Drop | Bucket::Duplicate => PetNoteState::Dropped,
    }
}

/// Rank a batch of unsurfaced notes. Duplicates (a fingerprint seen in the
/// last 7 days, or repeated earlier in this batch by `created_at`) are dropped.
/// Notify candidates are taken in `(score desc, created_at asc, id asc)` order
/// and demoted to the digest when flagged for prompt injection, inside quiet
/// hours, or once the daily budget is spent.
pub(crate) fn rank<Tz: TimeZone>(notes: &[PetNote], ctx: &RankCtx<'_, Tz>) -> Vec<SurfacedNote> {
    let mut ordered: Vec<&PetNote> = notes.iter().collect();
    ordered.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

    let mut batch_seen: HashSet<&str> = HashSet::new();
    let mut out: Vec<(SurfacedNote, &PetNote)> = Vec::with_capacity(ordered.len());
    for note in ordered {
        let s = score(note, ctx);
        let fp = note.fingerprint.as_str();
        let bucket = if ctx.seen_fingerprints.contains(fp) || !batch_seen.insert(fp) {
            Bucket::Duplicate
        } else {
            bucket_for_score(s)
        };
        out.push((
            SurfacedNote {
                note_id: note.id.clone(),
                score: s,
                bucket,
                state: state_for(bucket),
            },
            note,
        ));
    }

    let local_now = ctx.now.with_timezone(ctx.tz).time();
    let quiet = in_quiet_hours(local_now, ctx.quiet.0, ctx.quiet.1);
    let mut candidates: Vec<usize> = (0..out.len())
        .filter(|i| out[*i].0.bucket == Bucket::Notify)
        .collect();
    candidates.sort_by(|a, b| {
        let (sa, na) = &out[*a];
        let (sb, nb) = &out[*b];
        sb.score
            .cmp(&sa.score)
            .then(na.created_at.cmp(&nb.created_at))
            .then(na.id.cmp(&nb.id))
    });
    let mut notified = ctx.notified_today;
    for i in candidates {
        let flagged = out[i].1.injection_flagged;
        if flagged || quiet || notified >= ctx.budget {
            out[i].0.bucket = Bucket::Digest;
            out[i].0.state = PetNoteState::Queued;
        } else {
            notified += 1;
        }
    }
    for (s, _) in &out {
        log::debug!(
            "[pet::surfacer] note_id={} score={} bucket={}",
            s.note_id,
            s.score,
            s.bucket.as_str()
        );
    }
    out.into_iter().map(|(s, _)| s).collect()
}

/// Next occurrence of local wall-clock `hhmm` strictly after `now`. On a DST
/// gap the next valid minute is used; on an overlap, the earliest instant.
pub(crate) fn next_local_occurrence<Tz: TimeZone>(
    now: DateTime<Utc>,
    hhmm: NaiveTime,
    tz: &Tz,
) -> DateTime<Utc> {
    let mut date = now.with_timezone(tz).date_naive();
    for _ in 0..4 {
        let mut naive = date.and_time(hhmm);
        for _ in 0..=180 {
            if let Some(dt) = tz.from_local_datetime(&naive).earliest() {
                let utc = dt.with_timezone(&Utc);
                if utc > now {
                    return utc;
                }
                break;
            }
            naive += Duration::minutes(1);
        }
        date = date.succ_opt().unwrap_or(date);
    }
    now + Duration::days(1)
}
