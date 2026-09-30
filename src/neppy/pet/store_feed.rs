//! Persistence for Pet digests, proposals, run history and retention (the
//! connection and schema live in [`super::store`]).

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::neppy::config::Config;

use super::store::{
    parse_opt_ts, parse_ts, ts, with_connection, KEEP_DIGESTS, KEEP_RUNS, NOTE_RETENTION_DAYS,
};
use super::types::{PetDigest, PetProposal, PetRunSummary, ProposalState};

// ── Digests ──────────────────────────────────────────────────────────────

fn map_digest(r: &Row<'_>) -> rusqlite::Result<(PetDigest, String)> {
    Ok((
        PetDigest {
            id: r.get(0)?,
            pet_id: r.get(1)?,
            created_at: Utc::now(),
            local_date: r.get(3)?,
            body_md: r.get(4)?,
            item_count: r.get::<_, i64>(5)?.max(0) as u32,
            withheld_count: r.get::<_, i64>(6)?.max(0) as u32,
        },
        r.get(2)?,
    ))
}

/// Store a digest and mark `note_ids` as digested, in one transaction.
pub(crate) fn insert_digest_and_mark(
    config: &Config,
    digest: &PetDigest,
    note_ids: &[String],
) -> Result<()> {
    with_connection(config, |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO pet_digests (id, pet_id, created_at, local_date, body_md, item_count,
                withheld_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                digest.id,
                digest.pet_id,
                ts(digest.created_at),
                digest.local_date,
                digest.body_md,
                digest.item_count as i64,
                digest.withheld_count as i64
            ],
        )?;
        for id in note_ids {
            tx.execute(
                "UPDATE pet_notes SET state = 'digested', digest_id = ?1
                 WHERE id = ?2 AND pet_id = ?3",
                params![digest.id, id, digest.pet_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
}

/// Newest-first digests.
pub(crate) fn list_digests(config: &Config, pet_id: &str, limit: usize) -> Result<Vec<PetDigest>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, pet_id, created_at, local_date, body_md, item_count, withheld_count
             FROM pet_digests WHERE pet_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![pet_id, limit as i64], map_digest)?;
        let mut out = Vec::new();
        for row in rows {
            let (mut d, created) = row?;
            d.created_at = parse_ts(&created)?;
            out.push(d);
        }
        Ok(out)
    })
}

// ── Proposals ────────────────────────────────────────────────────────────

const PROPOSAL_SELECT: &str = "SELECT p.id, p.pet_id, p.note_id, n.title, p.action_text, \
     p.state, p.created_at, p.decided_at, p.expires_at \
     FROM pet_proposals p JOIN pet_notes n ON n.id = p.note_id";

fn map_proposal(r: &Row<'_>) -> rusqlite::Result<[Option<String>; 9]> {
    Ok([
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
    ])
}

fn proposal_from(cols: [Option<String>; 9]) -> Result<PetProposal> {
    let [id, pet_id, note_id, title, action, state, created, decided, expires] = cols;
    Ok(PetProposal {
        id: id.unwrap_or_default(),
        pet_id: pet_id.unwrap_or_default(),
        note_id: note_id.unwrap_or_default(),
        note_title: title.unwrap_or_default(),
        action_text: action.unwrap_or_default(),
        state: state
            .as_deref()
            .and_then(ProposalState::parse)
            .unwrap_or(ProposalState::Expired),
        created_at: parse_ts(created.as_deref().unwrap_or_default())?,
        decided_at: parse_opt_ts(decided)?,
        expires_at: parse_ts(expires.as_deref().unwrap_or_default())?,
    })
}

/// Move pending proposals past `expires_at` to `expired`.
fn expire_proposals(conn: &Connection, pet_id: &str, now: DateTime<Utc>) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE pet_proposals SET state = 'expired'
         WHERE pet_id = ?1 AND state = 'pending' AND expires_at <= ?2",
        params![pet_id, ts(now)],
    )?)
}

/// Pending proposals (lazily expiring stale ones first), newest first.
pub(crate) fn list_pending_proposals(
    config: &Config,
    pet_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<PetProposal>> {
    with_connection(config, |conn| {
        let expired = expire_proposals(conn, pet_id, now)?;
        if expired > 0 {
            log::debug!("[pet::store] expired {expired} proposals");
        }
        let sql = format!(
            "{PROPOSAL_SELECT} WHERE p.pet_id = ?1 AND p.state = 'pending'
             ORDER BY p.created_at DESC, p.id DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![pet_id], map_proposal)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(proposal_from(row?)?);
        }
        Ok(out)
    })
}

pub(crate) fn get_proposal(config: &Config, id: &str) -> Result<Option<PetProposal>> {
    with_connection(config, |conn| {
        let sql = format!("{PROPOSAL_SELECT} WHERE p.id = ?1");
        let cols = conn.query_row(&sql, params![id], map_proposal).optional()?;
        cols.map(proposal_from).transpose()
    })
}

/// Decide a still-pending, unexpired proposal. Returns `false` when it was
/// not pending (already decided or expired).
pub(crate) fn decide_proposal(
    config: &Config,
    id: &str,
    new_state: ProposalState,
    now: DateTime<Utc>,
) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn.execute(
            "UPDATE pet_proposals SET state = ?1, decided_at = ?2
             WHERE id = ?3 AND state = 'pending' AND expires_at > ?2",
            params![new_state.as_str(), ts(now), id],
        )?;
        Ok(changed > 0)
    })
}

// ── Runs ─────────────────────────────────────────────────────────────────

/// Record a finished pass; returns the run id.
pub(crate) fn insert_run(
    config: &Config,
    pet_id: &str,
    job_id: Option<&str>,
    run: &PetRunSummary,
    finished_at: DateTime<Utc>,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO pet_runs (id, pet_id, job_id, trigger, finished_at, success, notes_seen,
                notified, queued, dropped, digest_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                id,
                pet_id,
                job_id,
                run.trigger,
                ts(finished_at),
                (run.status == "completed") as i64,
                run.notes_seen as i64,
                run.notified as i64,
                run.queued as i64,
                run.dropped as i64,
                run.digest_id
            ],
        )?;
        Ok(())
    })?;
    Ok(id)
}

pub(crate) fn last_run(config: &Config, pet_id: &str) -> Result<Option<PetRunSummary>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT id, trigger, finished_at, success, notes_seen, notified, queued, dropped,
                        digest_id
                 FROM pet_runs WHERE pet_id = ?1 ORDER BY finished_at DESC, id DESC LIMIT 1",
                params![pet_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        [
                            r.get::<_, i64>(4)?,
                            r.get::<_, i64>(5)?,
                            r.get::<_, i64>(6)?,
                            r.get::<_, i64>(7)?,
                        ],
                        r.get::<_, Option<String>>(8)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(id, trigger, finished, success, counts, digest_id)| {
            Ok(PetRunSummary {
                run_id: Some(id),
                status: if success != 0 { "completed" } else { "failed" }.into(),
                trigger,
                finished_at: Some(parse_ts(&finished)?),
                notes_seen: counts[0].max(0) as u32,
                notified: counts[1].max(0) as u32,
                queued: counts[2].max(0) as u32,
                dropped: counts[3].max(0) as u32,
                digest_id,
            })
        })
        .transpose()
    })
}

/// Retention: drop terminal notes older than 30 days, keep the newest 200
/// runs and 60 digests, and expire stale proposals.
pub(crate) fn prune(config: &Config, pet_id: &str, now: DateTime<Utc>) -> Result<()> {
    let cutoff = ts(now - chrono::Duration::days(NOTE_RETENTION_DAYS));
    with_connection(config, |conn| {
        let tx = conn.transaction()?;
        let notes = tx.execute(
            "DELETE FROM pet_notes WHERE pet_id = ?1
               AND state IN ('dropped', 'digested', 'dismissed') AND created_at < ?2",
            params![pet_id, cutoff],
        )?;
        let runs = tx.execute(
            "DELETE FROM pet_runs WHERE pet_id = ?1 AND id NOT IN (
               SELECT id FROM pet_runs WHERE pet_id = ?1
               ORDER BY finished_at DESC, id DESC LIMIT ?2)",
            params![pet_id, KEEP_RUNS],
        )?;
        let digests = tx.execute(
            "DELETE FROM pet_digests WHERE pet_id = ?1 AND id NOT IN (
               SELECT id FROM pet_digests WHERE pet_id = ?1
               ORDER BY created_at DESC, id DESC LIMIT ?2)",
            params![pet_id, KEEP_DIGESTS],
        )?;
        let expired = expire_proposals(&tx, pet_id, now)?;
        tx.commit()?;
        log::debug!(
            "[pet::store] prune notes={notes} runs={runs} digests={digests} proposals_expired={expired}"
        );
        Ok(())
    })
}
