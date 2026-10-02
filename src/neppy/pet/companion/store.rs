//! Persistence for the companion: `{workspace}/pet/companion.db`, a separate
//! file from the research `pet.db` so that schema is untouched and "delete all"
//! is trivial.
//!
//! Tables: `companion_settings` (one JSON row), `companion_suggestions`,
//! `companion_actions`. Only scrubbed, capped excerpts are stored: every text
//! field is re-scrubbed on the way in (idempotent), so even a caller bug cannot
//! persist a secret. Raw observations are never stored (PC3), and nothing here
//! logs content.

use std::fmt;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use uuid::Uuid;

use super::sensitive::{scrub, ScrubCtx};
use super::settings::{
    apply_patch, CompanionSettings, CompanionSettingsPatch, ACTION_RETENTION_DAYS,
};
use super::types::*;
use super::usefulness::UsefulnessHistory;
use crate::neppy::config::Config;

pub const SCHEMA_VERSION: i64 = 1;
pub const MAX_TITLE_EXCERPT: usize = 200;
pub const MAX_CONTEXT_EXCERPT: usize = 1500;
pub const MAX_HEADLINE: usize = 140;
pub const MAX_BODY: usize = 1500;
pub const MAX_RESULT_EXCERPT: usize = 600;
/// Placeholder stored when a field is dropped by the scrubber.
pub const WITHHELD: &str = "[withheld]";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS companion_settings (
  id INTEGER PRIMARY KEY CHECK (id = 1), json TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS companion_suggestions (
  id TEXT PRIMARY KEY, created_at TEXT NOT NULL, trigger TEXT NOT NULL, kind TEXT NOT NULL,
  category TEXT NOT NULL, app_name TEXT NOT NULL, bundle_id TEXT, title_excerpt TEXT NOT NULL,
  context_excerpt TEXT NOT NULL, headline TEXT NOT NULL, body TEXT, state TEXT NOT NULL,
  score INTEGER NOT NULL, fingerprint TEXT NOT NULL, actions_json TEXT NOT NULL,
  handoff_thread_id TEXT, handoff_status TEXT, handoff_result TEXT
);
CREATE INDEX IF NOT EXISTS idx_companion_sugg_created ON companion_suggestions(created_at);
CREATE INDEX IF NOT EXISTS idx_companion_sugg_fp ON companion_suggestions(fingerprint, created_at);
CREATE TABLE IF NOT EXISTS companion_actions (
  id TEXT PRIMARY KEY, at TEXT NOT NULL, suggestion_id TEXT, category TEXT NOT NULL,
  decision TEXT NOT NULL, level INTEGER NOT NULL, outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_companion_actions_at ON companion_actions(at);
";

/// Fixed-width RFC3339 so timestamps compare correctly as strings in SQL.
fn ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_ts(raw: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("invalid stored timestamp '{raw}'"))?
        .with_timezone(&Utc))
}

/// Open `{workspace}/pet/companion.db`, apply the schema, run `f`.
pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&mut Connection) -> Result<T>,
) -> Result<T> {
    let path = config.workspace_dir.join("pet").join("companion.db");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create pet directory: {}", parent.display()))?;
    }
    let mut conn = Connection::open(&path)
        .with_context(|| format!("Failed to open companion DB: {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(SCHEMA)
        .context("Failed to initialize companion schema")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        log::debug!("[pet::companion] store schema user_version {version} -> {SCHEMA_VERSION}");
        conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    }
    f(&mut conn)
}

// ---- settings ----

fn read_settings(conn: &Connection) -> Result<CompanionSettings> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT json FROM companion_settings WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match raw {
        Some(json) => match serde_json::from_str::<CompanionSettings>(&json) {
            Ok(s) => s.normalized(),
            Err(e) => {
                // Fail closed to the defaults (companion OFF) on a corrupt row.
                log::warn!("[pet::companion] stored settings unreadable, using defaults: {e}");
                CompanionSettings::default()
            }
        },
        None => CompanionSettings::default(),
    })
}

fn write_settings(conn: &Connection, s: &CompanionSettings) -> Result<()> {
    conn.execute(
        "INSERT INTO companion_settings (id, json, updated_at) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
        params![serde_json::to_string(s)?, ts(Utc::now())],
    )?;
    Ok(())
}

/// Load settings (defaults when nothing is stored: companion OFF).
pub fn load_settings(config: &Config) -> Result<CompanionSettings> {
    with_connection(config, |c| read_settings(c))
}

pub fn save_settings(config: &Config, settings: &CompanionSettings) -> Result<()> {
    with_connection(config, |c| {
        write_settings(c, &settings.clone().normalized())
    })
}

/// Why [`update_settings`] failed.
#[derive(Debug)]
pub enum UpdateError {
    /// The patch is invalid; the message names the field.
    Invalid(String),
    Store(anyhow::Error),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Invalid(m) => f.write_str(m),
            UpdateError::Store(e) => write!(f, "{e:#}"),
        }
    }
}

impl From<anyhow::Error> for UpdateError {
    fn from(e: anyhow::Error) -> Self {
        UpdateError::Store(e)
    }
}

/// Validate and apply a patch atomically; returns the new settings.
pub fn update_settings(
    config: &Config,
    patch: &CompanionSettingsPatch,
) -> Result<CompanionSettings, UpdateError> {
    with_connection(config, |conn| {
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current = read_settings(&tx)?;
        match apply_patch(&current, patch) {
            Ok(next) => {
                let next = next.normalized();
                write_settings(&tx, &next)?;
                tx.commit()?;
                log::info!(
                    "[pet::companion] settings updated enabled={} level={}",
                    next.enabled,
                    next.level.name()
                );
                Ok(Ok(next))
            }
            Err(msg) => Ok(Err(msg)),
        }
    })
    .map_err(UpdateError::Store)?
    .map_err(UpdateError::Invalid)
}

// ---- suggestions ----

/// A suggestion to insert. Text fields are scrubbed again on insert.
#[derive(Debug, Clone)]
pub struct NewSuggestion {
    pub trigger: SuggestionTrigger,
    pub kind: TriggerKind,
    pub category: ActionCategory,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub title_excerpt: String,
    pub context_excerpt: String,
    pub headline: String,
    pub body: Option<String>,
    pub score: i32,
    pub actions: Vec<SuggestionAction>,
    pub fingerprint: String,
    pub now: DateTime<Utc>,
}

fn clean(text: &str, cap: usize) -> String {
    match scrub(text, &ScrubCtx::generated().capped(cap)).into_text() {
        Some(s) => s.into_string(),
        None => WITHHELD.to_string(),
    }
}

const SUGG_COLS: &str = "id, created_at, trigger, kind, category, app_name, bundle_id, \
    title_excerpt, context_excerpt, headline, body, state, score, actions_json, \
    handoff_thread_id, handoff_status, handoff_result";

fn row_to_suggestion(r: &Row<'_>) -> rusqlite::Result<CompanionSuggestion> {
    let bad = |what: &str, v: String| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("invalid stored {what} '{v}'").into(),
        )
    };
    let created: String = r.get(1)?;
    let trigger: String = r.get(2)?;
    let kind: String = r.get(3)?;
    let category: String = r.get(4)?;
    let state: String = r.get(11)?;
    let actions_json: String = r.get(13)?;
    let thread: Option<String> = r.get(14)?;
    let status: Option<String> = r.get(15)?;
    let result: Option<String> = r.get(16)?;
    let handoff = match (thread, status) {
        (Some(thread_id), Some(status)) => Some(Handoff {
            thread_id,
            status: HandoffStatus::parse(&status).ok_or_else(|| bad("handoff status", status))?,
            result_excerpt: result,
        }),
        _ => None,
    };
    let actions: Vec<String> = serde_json::from_str(&actions_json).unwrap_or_default();
    Ok(CompanionSuggestion {
        id: r.get(0)?,
        created_at: parse_ts(&created).map_err(|_| bad("timestamp", created.clone()))?,
        trigger: SuggestionTrigger::parse(&trigger).ok_or_else(|| bad("trigger", trigger))?,
        kind: TriggerKind::parse(&kind).ok_or_else(|| bad("kind", kind))?,
        category: ActionCategory::parse(&category).ok_or_else(|| bad("category", category))?,
        app_name: r.get(5)?,
        bundle_id: r.get(6)?,
        title_excerpt: r.get(7)?,
        context_excerpt: r.get(8)?,
        headline: r.get(9)?,
        body: r.get(10)?,
        state: SuggestionState::parse(&state).ok_or_else(|| bad("state", state))?,
        score: r.get(12)?,
        actions: actions
            .iter()
            .filter_map(|a| SuggestionAction::parse(a))
            .collect(),
        handoff,
    })
}

fn query_suggestions(
    conn: &Connection,
    sql: &str,
    p: &[&dyn rusqlite::ToSql],
) -> Result<Vec<CompanionSuggestion>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(p, row_to_suggestion)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn insert_suggestion(config: &Config, new: &NewSuggestion) -> Result<CompanionSuggestion> {
    let id = Uuid::new_v4().to_string();
    let title = clean(&new.title_excerpt, MAX_TITLE_EXCERPT);
    let context = clean(&new.context_excerpt, MAX_CONTEXT_EXCERPT);
    let headline = clean(&new.headline, MAX_HEADLINE);
    let body = new.body.as_deref().map(|b| clean(b, MAX_BODY));
    let actions: Vec<&str> = new.actions.iter().map(|a| a.as_str()).collect();
    with_connection(config, |c| {
        c.execute(
            "INSERT INTO companion_suggestions (id, created_at, trigger, kind, category, app_name,
               bundle_id, title_excerpt, context_excerpt, headline, body, state, score,
               fingerprint, actions_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'new',?12,?13,?14)",
            params![
                id,
                ts(new.now),
                new.trigger.as_str(),
                new.kind.as_str(),
                new.category.as_str(),
                new.app_name,
                new.bundle_id.as_deref().map(str::to_lowercase),
                title,
                context,
                headline,
                body,
                new.score,
                new.fingerprint,
                serde_json::to_string(&actions)?,
            ],
        )?;
        log::info!(
            "[pet::companion] suggestion created id={id} kind={} score={}",
            new.kind.as_str(),
            new.score
        );
        get_in(c, &id)?.context("inserted suggestion vanished")
    })
}

fn get_in(conn: &Connection, id: &str) -> Result<Option<CompanionSuggestion>> {
    Ok(query_suggestions(
        conn,
        &format!("SELECT {SUGG_COLS} FROM companion_suggestions WHERE id = ?1"),
        &[&id],
    )?
    .into_iter()
    .next())
}

pub fn get_suggestion(config: &Config, id: &str) -> Result<Option<CompanionSuggestion>> {
    with_connection(config, |c| get_in(c, id))
}

/// Newest first, optionally only those created before `before`.
pub fn list_suggestions(
    config: &Config,
    limit: u32,
    before: Option<DateTime<Utc>>,
) -> Result<Vec<CompanionSuggestion>> {
    let limit = limit.clamp(1, 100) as i64;
    with_connection(config, |c| {
        let before = ts(before.unwrap_or_else(|| Utc::now() + Duration::days(1)));
        query_suggestions(
            c,
            &format!(
                "SELECT {SUGG_COLS} FROM companion_suggestions WHERE created_at < ?1
                 ORDER BY created_at DESC, id DESC LIMIT ?2"
            ),
            &[&before, &limit],
        )
    })
}

pub fn set_state(config: &Config, id: &str, state: SuggestionState) -> Result<bool> {
    with_connection(config, |c| {
        Ok(c.execute(
            "UPDATE companion_suggestions SET state = ?2 WHERE id = ?1",
            params![id, state.as_str()],
        )? > 0)
    })
}

/// Set the generated body (scrubbed again).
pub fn set_body(config: &Config, id: &str, body: &str) -> Result<bool> {
    let body = clean(body, MAX_BODY);
    with_connection(config, |c| {
        Ok(c.execute(
            "UPDATE companion_suggestions SET body = ?2 WHERE id = ?1",
            params![id, body],
        )? > 0)
    })
}

pub fn set_handoff(config: &Config, id: &str, handoff: &Handoff) -> Result<bool> {
    let result = handoff
        .result_excerpt
        .as_deref()
        .map(|r| clean(r, MAX_RESULT_EXCERPT));
    with_connection(config, |c| {
        Ok(c.execute(
            "UPDATE companion_suggestions SET handoff_thread_id = ?2, handoff_status = ?3,
               handoff_result = ?4 WHERE id = ?1",
            params![id, handoff.thread_id, handoff.status.as_str(), result],
        )? > 0)
    })
}

pub fn delete_suggestion(config: &Config, id: &str) -> Result<bool> {
    with_connection(config, |c| {
        let n = c.execute("DELETE FROM companion_suggestions WHERE id = ?1", [id])?;
        c.execute(
            "UPDATE companion_actions SET suggestion_id = NULL WHERE suggestion_id = ?1",
            [id],
        )?;
        Ok(n > 0)
    })
}

/// Fingerprints surfaced in the last 30 min and dismissals in the last 24 h,
/// for the usefulness score.
pub fn usefulness_history(config: &Config, now: DateTime<Utc>) -> Result<UsefulnessHistory> {
    let novelty = ts(now - Duration::minutes(super::usefulness::NOVELTY_WINDOW_MIN));
    let dismiss = ts(now - Duration::hours(super::usefulness::DISMISS_WINDOW_HOURS));
    with_connection(config, |c| {
        let mut h = UsefulnessHistory::default();
        let mut stmt = c.prepare(
            "SELECT fingerprint, created_at FROM companion_suggestions WHERE created_at >= ?1",
        )?;
        for r in stmt.query_map([&novelty], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (fp, at) = r?;
            h.seen.push((fp, parse_ts(&at)?));
        }
        let mut stmt = c.prepare(
            "SELECT kind, COALESCE(bundle_id, ''), created_at FROM companion_suggestions
             WHERE state = 'dismissed' AND created_at >= ?1",
        )?;
        for r in stmt.query_map([&dismiss], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (kind, bundle, at) = r?;
            if let Some(kind) = TriggerKind::parse(&kind) {
                h.dismissals
                    .push((kind, bundle.to_lowercase(), parse_ts(&at)?));
            }
        }
        Ok(h)
    })
}

/// Proactive suggestions since `since` as `(created_at, app key)`, to seed the
/// rate limiter after a restart.
pub fn recent_proactive(
    config: &Config,
    since: DateTime<Utc>,
) -> Result<Vec<(DateTime<Utc>, String)>> {
    with_connection(config, |c| {
        let mut stmt = c.prepare(
            "SELECT created_at, COALESCE(bundle_id, lower(app_name)) FROM companion_suggestions
             WHERE trigger = 'proactive' AND created_at >= ?1 ORDER BY created_at",
        )?;
        let mut out = Vec::new();
        for r in stmt.query_map([ts(since)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (at, app) = r?;
            out.push((parse_ts(&at)?, app));
        }
        Ok(out)
    })
}

// ---- action audit ----

#[derive(Debug, Clone)]
pub struct NewAction {
    pub suggestion_id: Option<String>,
    pub category: ActionCategory,
    pub decision: ActionDecision,
    pub level: CompanionLevel,
    pub outcome: ActionOutcome,
    pub at: DateTime<Utc>,
}

pub fn insert_action(config: &Config, new: &NewAction) -> Result<CompanionActionLog> {
    let id = Uuid::new_v4().to_string();
    with_connection(config, |c| {
        c.execute(
            "INSERT INTO companion_actions (id, at, suggestion_id, category, decision, level, outcome)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                id,
                ts(new.at),
                new.suggestion_id,
                new.category.as_str(),
                new.decision.as_str(),
                new.level.as_u8(),
                new.outcome.as_str(),
            ],
        )?;
        log::info!(
            "[pet::companion] action logged category={} decision={} level={}",
            new.category.as_str(),
            new.decision.as_str(),
            new.level.as_u8()
        );
        Ok(CompanionActionLog {
            id,
            at: new.at,
            suggestion_id: new.suggestion_id.clone(),
            category: new.category,
            decision: new.decision,
            level: new.level,
            outcome: new.outcome,
        })
    })
}

pub fn list_actions(config: &Config, limit: u32) -> Result<Vec<CompanionActionLog>> {
    let limit = limit.clamp(1, 500) as i64;
    with_connection(config, |c| {
        let mut stmt = c.prepare(
            "SELECT id, at, suggestion_id, category, decision, level, outcome
             FROM companion_actions ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (id, at, suggestion_id, cat, dec, lvl, outcome) = r?;
            let parsed = (
                ActionCategory::parse(&cat),
                ActionDecision::parse(&dec),
                u8::try_from(lvl).ok().and_then(CompanionLevel::from_u8),
                ActionOutcome::parse(&outcome),
            );
            if let (Some(category), Some(decision), Some(level), Some(outcome)) = parsed {
                out.push(CompanionActionLog {
                    id,
                    at: parse_ts(&at)?,
                    suggestion_id,
                    category,
                    decision,
                    level,
                    outcome,
                });
            }
        }
        Ok(out)
    })
}

// ---- counts, prune, delete ----

/// `(suggestions, actions)` row counts.
pub fn counts(config: &Config) -> Result<(u64, u64)> {
    with_connection(config, |c| {
        let s: i64 = c.query_row("SELECT COUNT(*) FROM companion_suggestions", [], |r| {
            r.get(0)
        })?;
        let a: i64 = c.query_row("SELECT COUNT(*) FROM companion_actions", [], |r| r.get(0))?;
        Ok((s as u64, a as u64))
    })
}

/// Delete suggestions older than `retention_days` and actions older than 30
/// days. Returns `(suggestions, actions)` deleted.
pub fn prune(config: &Config, retention_days: u32, now: DateTime<Utc>) -> Result<(u64, u64)> {
    let s_cut = ts(now - Duration::days(retention_days as i64));
    let a_cut = ts(now - Duration::days(ACTION_RETENTION_DAYS));
    with_connection(config, |c| {
        let s = c.execute(
            "DELETE FROM companion_suggestions WHERE created_at < ?1",
            [s_cut],
        )?;
        let a = c.execute("DELETE FROM companion_actions WHERE at < ?1", [a_cut])?;
        if s + a > 0 {
            log::debug!("[pet::companion] pruned suggestions={s} actions={a}");
        }
        Ok((s as u64, a as u64))
    })
}

/// Delete every suggestion and audit row, then `VACUUM` so the freed pages do
/// not keep deleted text. Settings are kept (they are not observed data).
pub fn delete_all(config: &Config) -> Result<(u64, u64)> {
    with_connection(config, |c| {
        let s = c.execute("DELETE FROM companion_suggestions", [])?;
        let a = c.execute("DELETE FROM companion_actions", [])?;
        c.execute_batch("VACUUM")?;
        log::info!("[pet::companion] all data deleted suggestions={s} actions={a}");
        Ok((s as u64, a as u64))
    })
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
