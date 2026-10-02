//! Post-pass surfacing: rank the pass's notes, publish interrupts, and build at
//! most one digest when it is due.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};
use uuid::Uuid;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::neppy::config::Config;

use super::digest::build_digest;
use super::store::{self, PetRow};
use super::store_feed;
use super::store_notes;
use super::surfacer::{self, RankCtx};
use super::types::{
    PetDigest, PetNoteState, PetRunSummary, PET_DIGEST_JOB_NAME, PET_PROACTIVE_SOURCE_PREFIX,
};

/// Per-pet surfacing lock so a scheduled pass and a manual one never rank the
/// same `new` notes concurrently.
pub(crate) fn surface_lock(pet_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        OnceLock::new();
    LOCKS
        .get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
        .lock()
        .entry(pet_id.to_string())
        .or_default()
        .clone()
}

/// `HH:MM` → time; `None` when malformed (validated on write, so only a
/// hand-edited DB reaches the fallback).
pub(crate) fn parse_hhmm(raw: &str) -> Option<NaiveTime> {
    if raw.len() != 5 {
        return None;
    }
    NaiveTime::parse_from_str(raw, "%H:%M").ok()
}

fn digest_time(pet: &PetRow) -> NaiveTime {
    parse_hhmm(&pet.digest_time).unwrap_or_else(|| NaiveTime::from_hms_opt(7, 0, 0).unwrap())
}

/// UTC instant of the most recent local midnight.
fn local_midnight_utc<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> DateTime<Utc> {
    let date = now.with_timezone(tz).date_naive();
    tz.from_local_datetime(&date.and_time(NaiveTime::MIN))
        .earliest()
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or(now - Duration::hours(24))
}

/// Origin classes (`AgentTurnOrigin::class()`) whose parks the Pet may offer.
///
/// An ALLOWLIST, so a new origin kind is not surfaced until someone decides it
/// should be. `WebChat`: the user's own chat. A row that is no longer chat-routed
/// (its card went away) is still shown on purpose: approving it only updates the
/// stored decision (the parked call is gone or the row is an orphan), so it is
/// harmless, and it keeps a pending approval findable. `GoalContinuation`: the
/// user's own autonomous goal run, which has no other surface.
/// `PetCompanion`: work the Pet's desktop companion started (a hand-off run),
/// whose approvals belong in the Pet by definition (see
/// [`PET_COMPANION_ORIGIN_CLASS`]).
const SURFACEABLE_ORIGIN_CLASSES: &[&str] = &[
    "WebChat",
    "TrustedAutomation(GoalContinuation)",
    PET_COMPANION_ORIGIN_CLASS,
];

/// `AgentTurnOrigin::class()` of a Pet desktop-companion turn. Its parks are
/// listed in the Pet inbox even when they are ALSO routed as a card on the
/// hand-off thread: the user's attention is on the Pet, not on a background
/// thread they may never open, and deciding in either place resolves the same
/// request (the other card then reads as already decided).
pub(crate) const PET_COMPANION_ORIGIN_CLASS: &str = "TrustedAutomation(PetCompanion)";

/// Whether a parked approval may be offered to the user from the Pet (inbox,
/// digest count, notification). Single source of truth for all three.
///
/// Rows with a flow context have their own surface; remote `ExternalChannel`
/// input (Telegram, Discord, ...) is untrusted and TTL-denies silently by
/// design; any origin not on [`SURFACEABLE_ORIGIN_CLASSES`], including an
/// unknown one (a row written before the column existed), is not surfaced.
pub(crate) fn is_pet_surfaceable(row: &crate::neppy::security::approval::PendingApproval) -> bool {
    row.source_context.is_none()
        && row
            .origin_class
            .as_deref()
            .is_some_and(|class| SURFACEABLE_ORIGIN_CLASSES.contains(&class))
}

/// [`background_approvals`] over explicit rows (the seam tests drive). Drops
/// rows already shown as a chat card, except Pet-companion rows (see
/// [`PET_COMPANION_ORIGIN_CLASS`]).
pub(crate) fn filter_background_approvals(
    rows: Vec<crate::neppy::security::approval::PendingApproval>,
    chat_routed: &HashSet<String>,
) -> Vec<crate::neppy::security::approval::PendingApproval> {
    rows.into_iter()
        .filter(|r| {
            is_pet_surfaceable(r)
                && (!chat_routed.contains(&r.request_id)
                    || r.origin_class.as_deref() == Some(PET_COMPANION_ORIGIN_CLASS))
        })
        .collect()
}

/// All undecided approvals plus the ids already routed to a chat card, or empty
/// when no gate is installed. Unfiltered: callers apply
/// [`filter_background_approvals`].
pub(crate) fn pending_with_routed() -> (
    Vec<crate::neppy::security::approval::PendingApproval>,
    HashSet<String>,
) {
    let Some(gate) = crate::neppy::security::approval::ApprovalGate::try_global() else {
        return (Vec::new(), HashSet::new());
    };
    let routed = gate.chat_routed_request_ids();
    match gate.list_pending() {
        Ok(rows) => (rows, routed),
        Err(e) => {
            log::warn!("[pet] listing pending approvals failed: {e}");
            (Vec::new(), HashSet::new())
        }
    }
}

/// Pending background approvals without a chat card (for digest footers and
/// the inbox), excluding remote-origin and flow parks (see
/// [`is_pet_surfaceable`]). Empty when no gate is installed.
pub(crate) fn background_approvals() -> Vec<crate::neppy::security::approval::PendingApproval> {
    let (rows, routed) = pending_with_routed();
    filter_background_approvals(rows, &routed)
}

/// Build and store a digest from the current candidates. Returns `None` when
/// there is nothing to digest.
pub(crate) fn build_and_store_digest<Tz: TimeZone>(
    config: &Config,
    pet: &PetRow,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Result<Option<PetDigest>>
where
    Tz::Offset: std::fmt::Display,
{
    let candidates = store_notes::notes_for_digest(config, &pet.id)?;
    let proposal_notes = store_notes::pending_proposal_note_ids(config, &pet.id, now)?;
    let pending_proposals = store_feed::list_pending_proposals(config, &pet.id, now)?.len();
    let pending_approvals = background_approvals().len();
    let Some(build) = build_digest(
        &pet.name,
        &candidates,
        &proposal_notes,
        pending_proposals,
        pending_approvals,
        now,
        tz,
    ) else {
        log::debug!("[pet::digest] nothing to digest");
        return Ok(None);
    };
    let digest = PetDigest {
        id: Uuid::new_v4().to_string(),
        pet_id: pet.id.clone(),
        created_at: now,
        local_date: now.with_timezone(tz).format("%Y-%m-%d").to_string(),
        body_md: build.body_md.clone(),
        item_count: build.shown.len() as u32,
        withheld_count: build.withheld.len() as u32,
    };
    store_feed::insert_digest_and_mark(config, &digest, &build.all_ids())?;
    log::info!(
        "[pet::digest] stored digest_id={} items={} withheld={}",
        digest.id,
        digest.item_count,
        digest.withheld_count
    );
    Ok(Some(digest))
}

/// Rank the pass's notes, publish interrupts, advance the digest clock and
/// (at most once) deliver a due digest. See the module docs of `pet`.
pub async fn surface_after_pass(
    config: &Config,
    pet_id: &str,
    job_id: Option<&str>,
    trigger: &str,
    pass_success: bool,
) -> Result<PetRunSummary> {
    let lock = surface_lock(pet_id);
    let _guard = lock.lock().await;
    log::info!(
        "[pet::surfacer] start pet_id={pet_id} job_id={} trigger={trigger}",
        job_id.unwrap_or("-")
    );
    let pet = store::get_pet(config, pet_id)?.context("pet not found")?;
    let now = Utc::now();
    let summary = surface_with_clock(config, &pet, job_id, trigger, pass_success, now, &Local)?;
    Ok(summary)
}

/// Clock- and timezone-injectable body of [`surface_after_pass`] (tests drive
/// it across simulated sleep gaps).
pub(crate) fn surface_with_clock<Tz: TimeZone>(
    config: &Config,
    pet: &PetRow,
    job_id: Option<&str>,
    trigger: &str,
    pass_success: bool,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Result<PetRunSummary>
where
    Tz::Offset: std::fmt::Display,
{
    let goals = store::list_goals(config, &pet.id)?;
    let fresh = store_notes::new_notes(config, &pet.id)?;
    let seen: HashSet<String> =
        store_notes::recent_fingerprints(config, &pet.id, now - Duration::days(7), 10_000)?
            .into_iter()
            .map(|(f, _)| f)
            .collect();
    let notified_today =
        store_notes::notified_count_since(config, &pet.id, local_midnight_utc(now, tz))?;
    let quiet = (
        parse_hhmm(&pet.quiet_start).unwrap_or(NaiveTime::MIN),
        parse_hhmm(&pet.quiet_end).unwrap_or(NaiveTime::MIN),
    );
    let ctx = RankCtx {
        now,
        tz,
        goals: &goals,
        seen_fingerprints: &seen,
        notified_today,
        budget: pet.notify_budget_per_day,
        quiet,
    };
    let ranked = surfacer::rank(&fresh, &ctx);
    store_notes::apply_surfacing(config, &ranked, now)?;

    let mut summary = PetRunSummary {
        run_id: None,
        status: if pass_success { "completed" } else { "failed" }.into(),
        trigger: trigger.into(),
        finished_at: Some(now),
        notes_seen: ranked.len() as u32,
        notified: 0,
        queued: 0,
        dropped: 0,
        digest_id: None,
    };
    for s in &ranked {
        match s.state {
            PetNoteState::Notified => {
                summary.notified += 1;
                if let Some(note) = fresh.iter().find(|n| n.id == s.note_id) {
                    BUS.publish(DomainEvent::PetNoteSurfaced {
                        pet_id: pet.id.clone(),
                        note_id: note.id.clone(),
                        title: note.title.clone(),
                        source: note.source.as_str().to_string(),
                    });
                }
            }
            PetNoteState::Queued => summary.queued += 1,
            _ => summary.dropped += 1,
        }
    }
    store::set_last_pass_at(config, &pet.id, now)?;

    let hhmm = digest_time(pet);
    match pet.next_digest_at {
        None => {
            let next = surfacer::next_local_occurrence(now, hhmm, tz);
            store::set_next_digest_at(config, &pet.id, Some(next))?;
        }
        Some(due) if now >= due => {
            if let Some(digest) = build_and_store_digest(config, pet, now, tz)? {
                summary.digest_id = Some(digest.id.clone());
                BUS.publish(DomainEvent::PetDigestReady {
                    pet_id: pet.id.clone(),
                    digest_id: digest.id.clone(),
                    item_count: digest.item_count,
                });
                BUS.publish(DomainEvent::ProactiveMessageRequested {
                    source: format!("{PET_PROACTIVE_SOURCE_PREFIX}{}", pet.id),
                    message: digest.body_md.clone(),
                    job_name: Some(PET_DIGEST_JOB_NAME.into()),
                });
            }
            // Always from `now`, never `due + 1 day`: two days asleep still
            // yield exactly one digest.
            let next = surfacer::next_local_occurrence(now, hhmm, tz);
            store::set_next_digest_at(config, &pet.id, Some(next))?;
        }
        Some(_) => {}
    }

    let run_id = store_feed::insert_run(config, &pet.id, job_id, &summary, now)?;
    summary.run_id = Some(run_id);
    store_feed::prune(config, &pet.id, now)?;
    log::info!(
        "[pet::surfacer] done pet_id={} seen={} notified={} queued={} dropped={} digest={}",
        pet.id,
        summary.notes_seen,
        summary.notified,
        summary.queued,
        summary.dropped,
        summary.digest_id.is_some()
    );
    Ok(summary)
}
