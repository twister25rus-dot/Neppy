//! Persistence for Pet mode: one SQLite file at `{workspace_dir}/pet/pet.db`,
//! opened per call with the schema applied idempotently (same shape as
//! `cron/store.rs` and `security/approval/store.rs`).
//!
//! This file owns the connection, the pet row and goals; notes live in
//! [`super::store_notes`], digests / proposals / runs in [`super::store_feed`].

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use crate::neppy::config::Config;

use super::types::{PetGoal, PetSource, ResearchPreset, DEFAULT_PET_NAME};

/// Schema version written to `PRAGMA user_version`.
pub(crate) const SCHEMA_VERSION: i64 = 1;
/// Proposals expire this long after creation.
pub(crate) const PROPOSAL_TTL_DAYS: i64 = 7;
/// Terminal notes older than this are pruned.
pub(crate) const NOTE_RETENTION_DAYS: i64 = 30;
pub(crate) const KEEP_RUNS: i64 = 200;
pub(crate) const KEEP_DIGESTS: i64 = 60;

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS pet (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, persona TEXT NOT NULL DEFAULT '',
  enabled INTEGER NOT NULL DEFAULT 0, research_preset TEXT NOT NULL DEFAULT 'standard',
  digest_time TEXT NOT NULL DEFAULT '07:00', quiet_start TEXT NOT NULL DEFAULT '22:00',
  quiet_end TEXT NOT NULL DEFAULT '07:00', notify_budget_per_day INTEGER NOT NULL DEFAULT 3,
  sources_json TEXT NOT NULL DEFAULT '[\"memory\",\"tasks\",\"composio\",\"web\"]',
  research_job_id TEXT, next_digest_at TEXT, last_pass_at TEXT,
  created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pet_goals (
  id TEXT PRIMARY KEY, pet_id TEXT NOT NULL REFERENCES pet(id) ON DELETE CASCADE,
  text TEXT NOT NULL, created_at TEXT NOT NULL, archived_at TEXT
);
CREATE TABLE IF NOT EXISTS pet_notes (
  id TEXT PRIMARY KEY, pet_id TEXT NOT NULL REFERENCES pet(id) ON DELETE CASCADE,
  job_id TEXT, source TEXT NOT NULL, kind TEXT NOT NULL, title TEXT NOT NULL,
  body TEXT NOT NULL DEFAULT '', urgency INTEGER NOT NULL DEFAULT 1, due_at TEXT,
  goal_ids_json TEXT NOT NULL DEFAULT '[]', proposed_action TEXT, fingerprint TEXT NOT NULL,
  injection_flagged INTEGER NOT NULL DEFAULT 0, score INTEGER, bucket TEXT,
  state TEXT NOT NULL DEFAULT 'new', digest_id TEXT, created_at TEXT NOT NULL,
  surfaced_at TEXT, notified_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_pet_notes_state ON pet_notes(pet_id, state, created_at);
CREATE INDEX IF NOT EXISTS idx_pet_notes_fp ON pet_notes(pet_id, fingerprint, created_at);
CREATE TABLE IF NOT EXISTS pet_digests (
  id TEXT PRIMARY KEY, pet_id TEXT NOT NULL REFERENCES pet(id) ON DELETE CASCADE,
  created_at TEXT NOT NULL, local_date TEXT NOT NULL, body_md TEXT NOT NULL,
  item_count INTEGER NOT NULL, withheld_count INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS pet_proposals (
  id TEXT PRIMARY KEY, pet_id TEXT NOT NULL REFERENCES pet(id) ON DELETE CASCADE,
  note_id TEXT NOT NULL REFERENCES pet_notes(id) ON DELETE CASCADE,
  action_text TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'pending',
  created_at TEXT NOT NULL, decided_at TEXT, expires_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pet_runs (
  id TEXT PRIMARY KEY, pet_id TEXT NOT NULL REFERENCES pet(id) ON DELETE CASCADE,
  job_id TEXT, trigger TEXT NOT NULL, finished_at TEXT NOT NULL, success INTEGER NOT NULL,
  notes_seen INTEGER NOT NULL, notified INTEGER NOT NULL, queued INTEGER NOT NULL,
  dropped INTEGER NOT NULL, digest_id TEXT
);
";

/// Fixed-width RFC3339 (`…​.ffffffZ`) so stored timestamps compare correctly as
/// strings in SQL.
pub(crate) fn ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub(crate) fn parse_ts(raw: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid stored timestamp '{raw}'"))?
        .with_timezone(&Utc))
}

pub(crate) fn parse_opt_ts(raw: Option<String>) -> Result<Option<DateTime<Utc>>> {
    raw.as_deref().map(parse_ts).transpose()
}

/// Open `{workspace}/pet/pet.db`, apply the schema, and run `f`.
pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&mut Connection) -> Result<T>,
) -> Result<T> {
    let db_path = config.workspace_dir.join("pet").join("pet.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create pet directory: {}", parent.display()))?;
    }
    let mut conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open pet DB: {}", db_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA)
        .context("Failed to initialize pet schema")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        log::debug!("[pet::store] schema user_version {version} -> {SCHEMA_VERSION}");
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    }
    f(&mut conn)
}

/// The persisted pet row (goals and the cron job's next run are joined in by `ops`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PetRow {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub enabled: bool,
    pub research_preset: ResearchPreset,
    pub digest_time: String,
    pub quiet_start: String,
    pub quiet_end: String,
    pub notify_budget_per_day: u32,
    pub sources: Vec<PetSource>,
    pub research_job_id: Option<String>,
    pub next_digest_at: Option<DateTime<Utc>>,
    pub last_pass_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A validated profile update (built by `ops` from a `PetProfilePatch`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PetUpdate {
    pub name: Option<String>,
    pub persona: Option<String>,
    pub enabled: Option<bool>,
    pub research_preset: Option<ResearchPreset>,
    pub digest_time: Option<String>,
    pub quiet_start: Option<String>,
    pub quiet_end: Option<String>,
    pub notify_budget_per_day: Option<u32>,
    pub sources: Option<Vec<PetSource>>,
}

const PET_COLS: &str = "id, name, persona, enabled, research_preset, digest_time, quiet_start, \
     quiet_end, notify_budget_per_day, sources_json, research_job_id, next_digest_at, \
     last_pass_at, created_at, updated_at";

fn map_pet(row: &Row<'_>) -> rusqlite::Result<PetRowRaw> {
    Ok(PetRowRaw {
        id: row.get(0)?,
        name: row.get(1)?,
        persona: row.get(2)?,
        enabled: row.get::<_, i64>(3)? != 0,
        research_preset: row.get(4)?,
        digest_time: row.get(5)?,
        quiet_start: row.get(6)?,
        quiet_end: row.get(7)?,
        notify_budget_per_day: row.get(8)?,
        sources_json: row.get(9)?,
        research_job_id: row.get(10)?,
        next_digest_at: row.get(11)?,
        last_pass_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

struct PetRowRaw {
    id: String,
    name: String,
    persona: String,
    enabled: bool,
    research_preset: String,
    digest_time: String,
    quiet_start: String,
    quiet_end: String,
    notify_budget_per_day: i64,
    sources_json: String,
    research_job_id: Option<String>,
    next_digest_at: Option<String>,
    last_pass_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl PetRowRaw {
    fn into_row(self) -> Result<PetRow> {
        let sources: Vec<String> = serde_json::from_str(&self.sources_json).unwrap_or_default();
        Ok(PetRow {
            id: self.id,
            name: self.name,
            persona: self.persona,
            enabled: self.enabled,
            research_preset: ResearchPreset::parse(&self.research_preset)
                .unwrap_or(ResearchPreset::Standard),
            digest_time: self.digest_time,
            quiet_start: self.quiet_start,
            quiet_end: self.quiet_end,
            notify_budget_per_day: self.notify_budget_per_day.clamp(0, 10) as u32,
            sources: sources.iter().filter_map(|s| PetSource::parse(s)).collect(),
            research_job_id: self.research_job_id,
            next_digest_at: parse_opt_ts(self.next_digest_at)?,
            last_pass_at: parse_opt_ts(self.last_pass_at)?,
            created_at: parse_ts(&self.created_at)?,
            updated_at: parse_ts(&self.updated_at)?,
        })
    }
}

fn query_pet(conn: &Connection, where_sql: &str, arg: &str) -> Result<Option<PetRow>> {
    let sql = format!("SELECT {PET_COLS} FROM pet {where_sql}");
    let raw = conn.query_row(&sql, params![arg], map_pet).optional()?;
    raw.map(PetRowRaw::into_row).transpose()
}

/// The primary (oldest) pet, if one exists. Never creates.
pub(crate) fn primary_pet(config: &Config) -> Result<Option<PetRow>> {
    with_connection(config, |conn| {
        query_pet(conn, "WHERE ?1 = ?1 ORDER BY created_at ASC LIMIT 1", "")
    })
}

pub(crate) fn get_pet(config: &Config, id: &str) -> Result<Option<PetRow>> {
    with_connection(config, |conn| query_pet(conn, "WHERE id = ?1", id))
}

/// Return the primary pet, creating it (disabled, defaults) when absent.
pub(crate) fn ensure_primary(config: &Config, now: DateTime<Utc>) -> Result<PetRow> {
    let created = with_connection(config, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM pet ORDER BY created_at ASC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let id = match exists {
            Some(id) => (id, false),
            None => {
                let id = Uuid::new_v4().to_string();
                tx.execute(
                    "INSERT INTO pet (id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                    params![id, DEFAULT_PET_NAME, ts(now)],
                )?;
                (id, true)
            }
        };
        tx.commit()?;
        Ok(id)
    })?;
    if created.1 {
        log::info!("[pet::store] created primary pet (disabled)");
    }
    get_pet(config, &created.0)?.context("primary pet vanished after ensure")
}

/// Apply a validated update and return the new row.
pub(crate) fn update_pet(
    config: &Config,
    id: &str,
    update: &PetUpdate,
    now: DateTime<Utc>,
) -> Result<PetRow> {
    let mut pet = get_pet(config, id)?.with_context(|| format!("pet '{id}' not found"))?;
    if let Some(v) = &update.name {
        pet.name = v.clone();
    }
    if let Some(v) = &update.persona {
        pet.persona = v.clone();
    }
    if let Some(v) = update.enabled {
        pet.enabled = v;
    }
    if let Some(v) = update.research_preset {
        pet.research_preset = v;
    }
    if let Some(v) = &update.digest_time {
        pet.digest_time = v.clone();
    }
    if let Some(v) = &update.quiet_start {
        pet.quiet_start = v.clone();
    }
    if let Some(v) = &update.quiet_end {
        pet.quiet_end = v.clone();
    }
    if let Some(v) = update.notify_budget_per_day {
        pet.notify_budget_per_day = v;
    }
    if let Some(v) = &update.sources {
        pet.sources = v.clone();
    }
    let sources: Vec<&str> = pet.sources.iter().map(|s| s.as_str()).collect();
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE pet SET name = ?1, persona = ?2, enabled = ?3, research_preset = ?4,
                digest_time = ?5, quiet_start = ?6, quiet_end = ?7, notify_budget_per_day = ?8,
                sources_json = ?9, updated_at = ?10 WHERE id = ?11",
            params![
                pet.name,
                pet.persona,
                pet.enabled as i64,
                pet.research_preset.as_str(),
                pet.digest_time,
                pet.quiet_start,
                pet.quiet_end,
                pet.notify_budget_per_day as i64,
                serde_json::to_string(&sources)?,
                ts(now),
                id
            ],
        )?;
        Ok(())
    })?;
    get_pet(config, id)?.context("pet vanished after update")
}

fn set_column(config: &Config, id: &str, column: &str, value: Option<String>) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            &format!("UPDATE pet SET {column} = ?1 WHERE id = ?2"),
            params![value, id],
        )?;
        Ok(())
    })
}

pub(crate) fn set_research_job_id(config: &Config, id: &str, job_id: Option<&str>) -> Result<()> {
    set_column(config, id, "research_job_id", job_id.map(str::to_string))
}

pub(crate) fn set_last_pass_at(config: &Config, id: &str, at: DateTime<Utc>) -> Result<()> {
    set_column(config, id, "last_pass_at", Some(ts(at)))
}

pub(crate) fn set_next_digest_at(
    config: &Config,
    id: &str,
    at: Option<DateTime<Utc>>,
) -> Result<()> {
    set_column(config, id, "next_digest_at", at.map(ts))
}

/// The pet that owns research job `job_id`, if any.
pub(crate) fn find_pet_by_job(config: &Config, job_id: &str) -> Result<Option<String>> {
    with_connection(config, |conn| {
        Ok(conn
            .query_row(
                "SELECT id FROM pet WHERE research_job_id = ?1 LIMIT 1",
                params![job_id],
                |r| r.get(0),
            )
            .optional()?)
    })
}

// ── Goals ────────────────────────────────────────────────────────────────

/// Active (non-archived) goals, oldest first.
pub(crate) fn list_goals(config: &Config, pet_id: &str) -> Result<Vec<PetGoal>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, text, created_at FROM pet_goals
             WHERE pet_id = ?1 AND archived_at IS NULL ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![pet_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, text, created_at) = row?;
            out.push(PetGoal {
                id,
                text,
                created_at: parse_ts(&created_at)?,
            });
        }
        Ok(out)
    })
}

pub(crate) fn add_goal(
    config: &Config,
    pet_id: &str,
    text: &str,
    now: DateTime<Utc>,
) -> Result<PetGoal> {
    let goal = PetGoal {
        id: Uuid::new_v4().to_string(),
        text: text.to_string(),
        created_at: now,
    };
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO pet_goals (id, pet_id, text, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![goal.id, pet_id, goal.text, ts(now)],
        )?;
        Ok(())
    })?;
    Ok(goal)
}

/// Archive an active goal. Returns whether a row changed.
pub(crate) fn archive_goal(
    config: &Config,
    pet_id: &str,
    goal_id: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn.execute(
            "UPDATE pet_goals SET archived_at = ?1
             WHERE id = ?2 AND pet_id = ?3 AND archived_at IS NULL",
            params![ts(now), goal_id, pet_id],
        )?;
        Ok(changed > 0)
    })
}
