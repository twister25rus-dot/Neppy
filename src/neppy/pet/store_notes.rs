//! Persistence for Pet research notes (the connection and schema live in
//! [`super::store`]).

use std::collections::HashSet;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};
use uuid::Uuid;

use crate::neppy::config::Config;

use super::store::{parse_opt_ts, parse_ts, ts, with_connection, PROPOSAL_TTL_DAYS};
use super::surfacer::SurfacedNote;
use super::types::{Bucket, PetNote, PetNoteKind, PetNoteSource, PetNoteState};

/// A validated note ready to insert (built by the `pet_note` tool).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NewNote {
    pub pet_id: String,
    pub job_id: Option<String>,
    pub source: PetNoteSource,
    pub kind: PetNoteKind,
    pub title: String,
    pub body: String,
    pub urgency: u8,
    pub due_at: Option<DateTime<Utc>>,
    pub goal_ids: Vec<String>,
    pub proposed_action: Option<String>,
    pub fingerprint: String,
    pub injection_flagged: bool,
}

const NOTE_COLS: &str = "id, pet_id, source, kind, title, body, urgency, due_at, goal_ids_json, \
     proposed_action, injection_flagged, score, bucket, state, digest_id, created_at, \
     surfaced_at, notified_at, fingerprint";

type RawNote = (
    [String; 6],
    i64,
    [Option<String>; 3],
    i64,
    Option<i64>,
    Option<String>,
    String,
    Option<String>,
    String,
    [Option<String>; 2],
    String,
);

fn map_note(r: &Row<'_>) -> rusqlite::Result<RawNote> {
    Ok((
        [
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
        ],
        r.get(6)?,
        [r.get(7)?, r.get(8)?, r.get(9)?],
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
        r.get(13)?,
        r.get(14)?,
        r.get(15)?,
        [r.get(16)?, r.get(17)?],
        r.get(18)?,
    ))
}

fn note_from(raw: RawNote) -> Result<PetNote> {
    let (
        [id, pet_id, source, kind, title, body],
        urgency,
        [due_at, goal_ids_json, proposed_action],
        flagged,
        score,
        bucket,
        state,
        digest_id,
        created_at,
        [surfaced_at, notified_at],
        fingerprint,
    ) = raw;
    Ok(PetNote {
        id,
        pet_id,
        source: PetNoteSource::parse(&source).unwrap_or(PetNoteSource::Other),
        kind: PetNoteKind::parse(&kind).unwrap_or(PetNoteKind::Fyi),
        title,
        body,
        urgency: urgency.clamp(0, 3) as u8,
        due_at: parse_opt_ts(due_at)?,
        goal_ids: goal_ids_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
            .unwrap_or_default(),
        proposed_action,
        fingerprint,
        injection_flagged: flagged != 0,
        score: score.map(|s| s.clamp(0, 100) as u8),
        bucket: bucket.as_deref().and_then(Bucket::parse),
        state: PetNoteState::parse(&state).unwrap_or(PetNoteState::New),
        digest_id,
        created_at: parse_ts(&created_at)?,
        surfaced_at: parse_opt_ts(surfaced_at)?,
        notified_at: parse_opt_ts(notified_at)?,
    })
}

fn query_notes(
    config: &Config,
    where_sql: &str,
    args: &[&dyn rusqlite::ToSql],
) -> Result<Vec<PetNote>> {
    with_connection(config, |conn| {
        let sql = format!("SELECT {NOTE_COLS} FROM pet_notes {where_sql}");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(args, map_note)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(note_from(row?)?);
        }
        Ok(out)
    })
}

/// Insert a note and, when `create_proposal` and the note carries a
/// `proposed_action`, its pending proposal — in one transaction. Returns the
/// stored note and the proposal id.
pub(crate) fn insert_note(
    config: &Config,
    note: &NewNote,
    create_proposal: bool,
    now: DateTime<Utc>,
) -> Result<(PetNote, Option<String>)> {
    let id = Uuid::new_v4().to_string();
    let proposal_id = with_connection(config, |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO pet_notes (id, pet_id, job_id, source, kind, title, body, urgency, due_at,
                goal_ids_json, proposed_action, fingerprint, injection_flagged, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 'new', ?14)",
            params![
                id,
                note.pet_id,
                note.job_id,
                note.source.as_str(),
                note.kind.as_str(),
                note.title,
                note.body,
                note.urgency as i64,
                note.due_at.map(ts),
                serde_json::to_string(&note.goal_ids)?,
                note.proposed_action,
                note.fingerprint,
                note.injection_flagged as i64,
                ts(now)
            ],
        )?;
        let proposal_id = match (&note.proposed_action, create_proposal) {
            (Some(action), true) => {
                let pid = Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO pet_proposals (id, pet_id, note_id, action_text, state,
                        created_at, expires_at) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
                    params![
                        pid,
                        note.pet_id,
                        id,
                        action,
                        ts(now),
                        ts(now + chrono::Duration::days(PROPOSAL_TTL_DAYS))
                    ],
                )?;
                Some(pid)
            }
            _ => None,
        };
        tx.commit()?;
        Ok(proposal_id)
    })?;
    let stored = get_note(config, &id)?.context("note vanished after insert")?;
    Ok((stored, proposal_id))
}

pub(crate) fn get_note(config: &Config, id: &str) -> Result<Option<PetNote>> {
    Ok(query_notes(config, "WHERE id = ?1", &[&id])?
        .into_iter()
        .next())
}

/// Unsurfaced notes this job recorded since `since` — the per-pass count.
pub(crate) fn count_pass_notes(
    config: &Config,
    pet_id: &str,
    job_id: &str,
    since: DateTime<Utc>,
) -> Result<usize> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pet_notes WHERE pet_id = ?1 AND job_id = ?2
               AND state = 'new' AND created_at >= ?3",
            params![pet_id, job_id, ts(since)],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as usize)
    })
}

/// Newest-first notes, optionally filtered by state, before a `created_at` cursor.
pub(crate) fn list_notes(
    config: &Config,
    pet_id: &str,
    states: &[PetNoteState],
    limit: usize,
    before: Option<DateTime<Utc>>,
) -> Result<Vec<PetNote>> {
    let before = ts(before.unwrap_or_else(|| Utc::now() + chrono::Duration::days(3650)));
    let state_filter = if states.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = states.iter().map(|s| format!("'{}'", s.as_str())).collect();
        format!("AND state IN ({})", list.join(", "))
    };
    let limit = limit as i64;
    query_notes(
        config,
        &format!(
            "WHERE pet_id = ?1 AND created_at < ?2 {state_filter}
             ORDER BY created_at DESC, id DESC LIMIT ?3"
        ),
        &[&pet_id, &before, &limit],
    )
}

/// Notes the surfacer has not ranked yet, oldest first.
pub(crate) fn new_notes(config: &Config, pet_id: &str) -> Result<Vec<PetNote>> {
    query_notes(
        config,
        "WHERE pet_id = ?1 AND state = 'new' ORDER BY created_at ASC, id ASC",
        &[&pet_id],
    )
}

/// `(fingerprint, title)` of already-surfaced notes created since `since`,
/// newest first.
pub(crate) fn recent_fingerprints(
    config: &Config,
    pet_id: &str,
    since: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<(String, String)>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT fingerprint, title FROM pet_notes
             WHERE pet_id = ?1 AND state != 'new' AND created_at >= ?2
             ORDER BY created_at DESC, id DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![pet_id, ts(since), limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    })
}

/// How many notes interrupted the user since `since`.
pub(crate) fn notified_count_since(
    config: &Config,
    pet_id: &str,
    since: DateTime<Utc>,
) -> Result<u32> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pet_notes WHERE pet_id = ?1 AND notified_at >= ?2",
            params![pet_id, ts(since)],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u32)
    })
}

/// Persist a ranking in one transaction. Only rows still in `new` move.
pub(crate) fn apply_surfacing(
    config: &Config,
    surfaced: &[SurfacedNote],
    now: DateTime<Utc>,
) -> Result<()> {
    with_connection(config, |conn| {
        let tx = conn.transaction()?;
        for s in surfaced {
            let notified_at = (s.state == PetNoteState::Notified).then(|| ts(now));
            tx.execute(
                "UPDATE pet_notes SET score = ?1, bucket = ?2, state = ?3, surfaced_at = ?4,
                    notified_at = COALESCE(?5, notified_at)
                 WHERE id = ?6 AND state = 'new'",
                params![
                    s.score as i64,
                    s.bucket.as_str(),
                    s.state.as_str(),
                    ts(now),
                    notified_at,
                    s.note_id
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
}

/// Digest candidates: queued notes plus notified notes not yet in a digest.
pub(crate) fn notes_for_digest(config: &Config, pet_id: &str) -> Result<Vec<PetNote>> {
    query_notes(
        config,
        "WHERE pet_id = ?1 AND (state = 'queued' OR (state = 'notified' AND digest_id IS NULL))
         ORDER BY created_at ASC, id ASC",
        &[&pet_id],
    )
}

/// Note ids that have a pending (unexpired) proposal.
pub(crate) fn pending_proposal_note_ids(
    config: &Config,
    pet_id: &str,
    now: DateTime<Utc>,
) -> Result<HashSet<String>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT note_id FROM pet_proposals
             WHERE pet_id = ?1 AND state = 'pending' AND expires_at > ?2",
        )?;
        let rows = stmt.query_map(params![pet_id, ts(now)], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
    })
}

/// Dismiss a note (and its pending proposal). `None` when the note is unknown.
pub(crate) fn dismiss_note(
    config: &Config,
    pet_id: &str,
    note_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<PetNote>> {
    let changed = with_connection(config, |conn| {
        let tx = conn.transaction()?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM pet_notes WHERE id = ?1 AND pet_id = ?2",
                params![note_id, pet_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Ok(false);
        }
        tx.execute(
            "UPDATE pet_notes SET state = 'dismissed' WHERE id = ?1",
            params![note_id],
        )?;
        tx.execute(
            "UPDATE pet_proposals SET state = 'dismissed', decided_at = ?1
             WHERE note_id = ?2 AND state = 'pending'",
            params![ts(now), note_id],
        )?;
        tx.commit()?;
        Ok(true)
    })?;
    if !changed {
        return Ok(None);
    }
    get_note(config, note_id)
}
