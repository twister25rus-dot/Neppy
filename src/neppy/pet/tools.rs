//! Agent tools private to the Pet research lane: `pet_context`,
//! `pet_recent_memory` and `pet_note`.
//!
//! All three refuse unless the ambient turn origin is
//! `TrustedAutomation { source: PetResearch, .. }`, so an ordinary chat or cron
//! turn that happens to see them (e.g. a wildcard agent) cannot write pet state.
//! None has an external effect; `pet_note` writes only the pet's own DB.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::neppy::agent::turn_origin::{self, AgentTurnOrigin, TrustedAutomationSource};
use crate::neppy::config::Config;
use crate::neppy::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::neppy::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::store;
use super::store_notes::{self, NewNote};
use super::types::{
    PetNoteKind, PetNoteSource, MAX_ACTION, MAX_BODY, MAX_FINGERPRINT, MAX_GOALS,
    MAX_NOTES_PER_PASS, MAX_TITLE,
};

pub(crate) const NOT_PET_ORIGIN: &str = "pet_* tools are only available to the Pet research pass";
/// A research pass never runs longer than this; notes recorded by the job in
/// this window (and not surfaced yet) count towards the per-pass cap.
pub(crate) const PASS_WINDOW_HOURS: i64 = 2;
/// Default memory window when the pet has never run.
pub(crate) const DEFAULT_WINDOW_HOURS: i64 = 24;
/// The memory window never reaches further back than this.
pub(crate) const MAX_WINDOW_HOURS: i64 = 72;

/// The research job id when the current turn is the Pet research lane.
pub(crate) fn pet_research_job_id() -> Option<String> {
    match turn_origin::current() {
        Some(AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::PetResearch,
            job_id,
        }) => Some(job_id),
        _ => None,
    }
}

/// `(pet_id, job_id)` for the current research turn, or the refusal result.
fn resolve_pet(config: &Config, tool: &str) -> Result<(String, String), ToolResult> {
    let Some(job_id) = pet_research_job_id() else {
        log::debug!("[pet::tools] {tool} refused: not a pet research origin");
        return Err(ToolResult::error(NOT_PET_ORIGIN));
    };
    match store::find_pet_by_job(config, &job_id) {
        Ok(Some(pet_id)) => Ok((pet_id, job_id)),
        Ok(None) => {
            log::warn!("[pet::tools] {tool}: no pet owns the running research job");
            Err(ToolResult::error("no Pet owns this research job"))
        }
        Err(e) => Err(ToolResult::error(format!("pet store unavailable: {e}"))),
    }
}

/// The memory window `pet_context` hands the agent: since the last pass
/// (fallback 24h), capped at 72h, until now.
pub(crate) fn memory_window(
    last_pass_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let floor = now - Duration::hours(MAX_WINDOW_HOURS);
    let since = last_pass_at
        .filter(|t| *t < now)
        .unwrap_or(now - Duration::hours(DEFAULT_WINDOW_HOURS));
    (since.max(floor), now)
}

// ── pet_context ──────────────────────────────────────────────────────────

pub struct PetContextTool {
    config: Arc<Config>,
}

impl PetContextTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for PetContextTool {
    fn name(&self) -> &str {
        "pet_context"
    }

    fn description(&self) -> &str {
        "Pet research pass only. Returns the pet's name and persona, the user's active goals, \
         enabled sources, the memory window to scan (since_ms/until_ms), fingerprints already \
         recorded in the last 7 days (skip those), and how many notes this pass has recorded."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let (pet_id, job_id) = match resolve_pet(&self.config, "pet_context") {
            Ok(v) => v,
            Err(refusal) => return Ok(refusal),
        };
        let now = Utc::now();
        let Some(pet) = store::get_pet(&self.config, &pet_id)? else {
            return Ok(ToolResult::error("pet not found"));
        };
        let goals = store::list_goals(&self.config, &pet_id)?;
        let (since, until) = memory_window(pet.last_pass_at, now);
        let recent =
            store_notes::recent_fingerprints(&self.config, &pet_id, now - Duration::days(7), 100)?;
        let recorded = store_notes::count_pass_notes(
            &self.config,
            &pet_id,
            &job_id,
            now - Duration::hours(PASS_WINDOW_HOURS),
        )?;
        log::debug!(
            "[pet::tools] pet_context goals={} recent={} recorded={recorded}",
            goals.len(),
            recent.len()
        );
        Ok(ToolResult::json(json!({
            "pet": { "name": pet.name, "persona": pet.persona },
            "goals": goals.iter().map(|g| json!({ "id": g.id, "text": g.text })).collect::<Vec<_>>(),
            "sources_enabled": pet.sources.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "window": { "since_ms": since.timestamp_millis(), "until_ms": until.timestamp_millis() },
            "recent_fingerprints": recent
                .iter()
                .map(|(f, t)| json!({ "fingerprint": f, "title": t }))
                .collect::<Vec<_>>(),
            "notes_recorded_this_pass": recorded,
            "max_notes_per_pass": MAX_NOTES_PER_PASS,
        })))
    }
}

// ── pet_recent_memory ────────────────────────────────────────────────────

/// Read-only windowed memory recap. Exists because the registered
/// `memory_tree` tool also has a write mode (`ingest_document`), so it cannot
/// be allowlisted for this lane.
pub struct PetRecentMemoryTool {
    config: Arc<Config>,
}

impl PetRecentMemoryTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for PetRecentMemoryTool {
    fn name(&self) -> &str {
        "pet_recent_memory"
    }

    fn description(&self) -> &str {
        "Pet research pass only. Read-only: return the minimum set of memory nodes covering \
         the window [since_ms, until_ms] from pet_context, grouped by source."
    }

    fn parameters_schema(&self) -> Value {
        crate::neppy::memory::query::MemoryTreeCoverWindowTool.parameters_schema()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        if let Err(refusal) = resolve_pet(&self.config, "pet_recent_memory") {
            return Ok(refusal);
        }
        log::debug!("[pet::tools] pet_recent_memory delegating to cover_window");
        crate::neppy::memory::query::MemoryTreeCoverWindowTool
            .execute(args)
            .await
    }
}

// ── pet_note ─────────────────────────────────────────────────────────────

pub struct PetNoteTool {
    config: Arc<Config>,
}

impl PetNoteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

fn clean(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn opt_str(args: &Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("invalid '{key}': expected a string")),
    }
}

fn bounded(key: &str, raw: Option<String>, max: usize) -> Result<Option<String>, String> {
    let Some(v) = raw.map(|s| clean(&s)).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if v.chars().count() > max {
        return Err(format!("invalid '{key}': at most {max} characters"));
    }
    Ok(Some(v))
}

/// Default dedupe key: first 16 hex chars of sha256(source \n normalized title).
pub(crate) fn default_fingerprint(source: PetNoteSource, title: &str) -> String {
    let normalized = title
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let digest = Sha256::digest(format!("{}\n{normalized}", source.as_str()).as_bytes());
    hex::encode(digest)[..16].to_string()
}

/// Parse and validate `pet_note` arguments (everything except the injection scan).
pub(crate) fn parse_note_args(args: &Value, pet_id: &str, job_id: &str) -> Result<NewNote, String> {
    let enum_err =
        |key: &str, all: Vec<&str>| format!("invalid '{key}': expected one of {}", all.join("|"));
    let source = opt_str(args, "source")?
        .and_then(|s| PetNoteSource::parse(&s))
        .ok_or_else(|| {
            enum_err(
                "source",
                PetNoteSource::ALL.iter().map(|s| s.as_str()).collect(),
            )
        })?;
    let kind = opt_str(args, "kind")?
        .and_then(|s| PetNoteKind::parse(&s))
        .ok_or_else(|| {
            enum_err(
                "kind",
                PetNoteKind::ALL.iter().map(|s| s.as_str()).collect(),
            )
        })?;
    let title = bounded("title", opt_str(args, "title")?, MAX_TITLE)?
        .ok_or_else(|| "invalid 'title': required".to_string())?;
    let body = bounded("body", opt_str(args, "body")?, MAX_BODY)?.unwrap_or_default();
    let urgency = match args.get("urgency") {
        None | Some(Value::Null) => 1,
        Some(v) => v
            .as_i64()
            .ok_or_else(|| "invalid 'urgency': expected an integer 0..3".to_string())?
            .clamp(0, 3) as u8,
    };
    let due_at = match opt_str(args, "due_at")? {
        None => None,
        Some(raw) => Some(
            DateTime::parse_from_rfc3339(raw.trim())
                .map_err(|_| "invalid 'due_at': expected an RFC3339 timestamp".to_string())?
                .with_timezone(&Utc),
        ),
    };
    let goal_ids = match args.get("goal_ids") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => {
            let ids: Vec<String> = items
                .iter()
                .filter_map(|v| v.as_str().map(clean))
                .filter(|s| !s.is_empty() && s.len() <= 64)
                .collect();
            if ids.len() > MAX_GOALS {
                return Err(format!("invalid 'goal_ids': at most {MAX_GOALS} ids"));
            }
            ids
        }
        Some(_) => return Err("invalid 'goal_ids': expected an array of strings".into()),
    };
    let proposed_action = bounded(
        "proposed_action",
        opt_str(args, "proposed_action")?,
        MAX_ACTION,
    )?;
    let fingerprint = bounded(
        "fingerprint",
        opt_str(args, "fingerprint")?,
        MAX_FINGERPRINT,
    )?
    .unwrap_or_else(|| default_fingerprint(source, &title));
    Ok(NewNote {
        pet_id: pet_id.to_string(),
        job_id: Some(job_id.to_string()),
        source,
        kind,
        title,
        body,
        urgency,
        due_at,
        goal_ids,
        proposed_action,
        fingerprint,
        injection_flagged: false,
    })
}

/// Whether the note's text trips the prompt-injection detector.
pub(crate) fn injection_flagged(note: &NewNote) -> bool {
    let text = format!(
        "{}\n{}\n{}",
        note.title,
        note.body,
        note.proposed_action.as_deref().unwrap_or_default()
    );
    let decision = enforce_prompt_input(
        &text,
        PromptEnforcementContext {
            source: "pet.note",
            request_id: None,
            user_id: None,
            session_id: None,
        },
    );
    decision.action != PromptEnforcementAction::Allow
}

#[async_trait]
impl Tool for PetNoteTool {
    fn name(&self) -> &str {
        "pet_note"
    }

    fn description(&self) -> &str {
        "Pet research pass only. Record one private finding for the user's Pet. Never sends \
         anything. To suggest an action, set kind=\"proposal\" (or any kind) with \
         proposed_action — the user decides later. Titles are factual, <= 140 chars, no URLs."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source": { "type": "string", "enum": ["memory","tasks","calendar","email","web","other"] },
                "kind": { "type": "string", "enum": ["deadline","request","meeting","change","fyi","idea","proposal"] },
                "title": { "type": "string", "maxLength": MAX_TITLE },
                "body": { "type": "string", "maxLength": MAX_BODY },
                "urgency": { "type": "integer", "minimum": 0, "maximum": 3,
                    "description": "3 = action within 24h or blocks someone; 2 = this week; 1 = worth knowing; 0 = background" },
                "due_at": { "type": "string", "description": "RFC3339 timestamp, when the finding has a deadline or start time" },
                "goal_ids": { "type": "array", "items": { "type": "string" } },
                "proposed_action": { "type": "string", "maxLength": MAX_ACTION },
                "fingerprint": { "type": "string", "maxLength": MAX_FINGERPRINT,
                    "description": "Stable source id such as email:<thread_id> or task:<id>" }
            },
            "required": ["source", "kind", "title", "urgency"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Writes only the pet's own DB; no external effect.
        PermissionLevel::Write
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let (pet_id, job_id) = match resolve_pet(&self.config, "pet_note") {
            Ok(v) => v,
            Err(refusal) => return Ok(refusal),
        };
        let now = Utc::now();
        let recorded = store_notes::count_pass_notes(
            &self.config,
            &pet_id,
            &job_id,
            now - Duration::hours(PASS_WINDOW_HOURS),
        )?;
        if recorded >= MAX_NOTES_PER_PASS {
            log::debug!("[pet::tools] pet_note refused: per-pass cap reached");
            return Ok(ToolResult::error(format!(
                "note cap reached: at most {MAX_NOTES_PER_PASS} notes per pass"
            )));
        }
        let mut note = match parse_note_args(&args, &pet_id, &job_id) {
            Ok(n) => n,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };
        note.injection_flagged = injection_flagged(&note);
        let create_proposal = note.proposed_action.is_some() && !note.injection_flagged;
        log::debug!(
            "[pet::tools] pet_note title_len={} body_len={} has_action={} flagged={}",
            note.title.chars().count(),
            note.body.chars().count(),
            note.proposed_action.is_some(),
            note.injection_flagged
        );
        let (stored, proposal_id) =
            store_notes::insert_note(&self.config, &note, create_proposal, now)?;
        Ok(ToolResult::json(json!({
            "note_id": stored.id,
            "proposal_id": proposal_id,
            "flagged": stored.injection_flagged,
        })))
    }
}
