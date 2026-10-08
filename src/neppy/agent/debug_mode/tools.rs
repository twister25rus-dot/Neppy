//! Agent tools for the Debug agent: `debug_checkpoint`, `debug_run_check`
//! (in [`super::tools_check`]), `debug_validate_candidate` and `debug_report`.
//!
//! All act on the debug task of the *current turn* ([`super::turn::current`]),
//! so they refuse outside a Debug-mode turn. Rollback is deliberately not a
//! tool: it stays a user-confirmed RPC.

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::neppy::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::candidate;
use super::ops;
use super::pass_gate;
use super::selfmod;
use super::turn::{self, DebugTurn};
use super::types::{TaskPatch, TaskStatus, ValidationRecord};

pub use super::tools_check::DebugRunCheckTool;

const NOT_DEBUG_TURN: &str = "debug_* tools only work inside a Debug-mode turn";
const NO_TASK: &str =
    "this debug turn has no recorded task (task bookkeeping failed at turn start)";
const MAX_VALIDATIONS: usize = 50;
/// How long `debug_validate_candidate` waits for the build + health check.
const VALIDATE_WAIT: Duration = Duration::from_secs(45 * 60);
const VALIDATE_POLL: Duration = Duration::from_secs(2);

pub(super) fn turn_with_task(tool: &str) -> Result<(DebugTurn, String), ToolResult> {
    let Some(turn) = turn::current() else {
        log::debug!("[debug_mode] {tool} refused: not a debug turn");
        return Err(ToolResult::error(NOT_DEBUG_TURN));
    };
    match turn.task_id.clone() {
        Some(id) => Ok((turn, id)),
        None => Err(ToolResult::error(NO_TASK)),
    }
}

// ── debug_checkpoint ────────────────────────────────────────────────────

/// Mid-task checkpoint of the working tree, tied to the current debug task.
pub struct DebugCheckpointTool;

#[async_trait]
impl Tool for DebugCheckpointTool {
    fn name(&self) -> &str {
        "debug_checkpoint"
    }

    fn description(&self) -> &str {
        "Debug mode only. Record a non-destructive checkpoint of the project working tree \
         (e.g. after a milestone such as 'provider abstraction complete') so the user can \
         roll back to it. A checkpoint was already taken automatically before this turn."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "What this checkpoint captures, e.g. 'IPC layer done, UI pending'."
                }
            },
            "required": ["description"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Writes git refs/objects under refs/neppy-debug/ only.
        PermissionLevel::Write
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let (turn, task_id) = match turn_with_task("debug_checkpoint") {
            Ok(v) => v,
            Err(refusal) => return Ok(refusal),
        };
        let Some(description) = args
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return Ok(ToolResult::error("`description` is required"));
        };
        let root = turn.root.display().to_string();
        match ops::checkpoint_create(&turn.ctx, Some(&root), description, Some(&task_id)).await {
            Ok(o) => {
                let cp = o.value;
                log::debug!("[debug_mode] debug_checkpoint id={} task={task_id}", cp.id);
                Ok(ToolResult::json(json!({
                    "checkpoint_id": cp.id,
                    "task_id": task_id,
                    "head": cp.head,
                    "dirty_files": cp.dirty_files.len(),
                    "untracked_files": cp.untracked_files.len(),
                })))
            }
            Err(e) => Ok(ToolResult::error(format!("checkpoint failed: {e}"))),
        }
    }
}

// ── debug_validate_candidate ─────────────────────────────────────────────

/// Cancels the candidate if the tool call is dropped (turn cancelled) before
/// it finished waiting, so an abandoned build does not keep burning CPU.
struct CancelOnDrop {
    ctx: Option<ops::DebugCtx>,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(ctx) = self.ctx.take() else { return };
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            h.spawn(async move {
                let _ = candidate::cancel(&ctx).await;
            });
        }
    }
}

/// Render a candidate record as the tool's JSON result.
fn candidate_json(rec: &super::types::CandidateRecord) -> Value {
    use super::types::CandidatePhase;
    json!({
        "candidate_id": rec.id,
        "phase": rec.phase,
        "passed": rec.phase == CandidatePhase::Passed,
        "build_ok": rec.build_ok,
        "health_ok": rec.health_ok,
        "tree_hash": rec.tree_hash,
        "error_tail": rec.error_tail,
        "known_good_saved": rec.known_good_path.is_some(),
    })
}

/// Staged self-update check (spec 35-36) for the current debug task.
pub struct DebugValidateCandidateTool;

#[async_trait]
impl Tool for DebugValidateCandidateTool {
    fn name(&self) -> &str {
        "debug_validate_candidate"
    }

    fn description(&self) -> &str {
        "Debug mode only. Validate your changes to the application's own core as a staged \
         candidate: builds the modified source into an isolated target dir, launches the \
         result as a separate process with a throwaway workspace, health-checks it, and \
         only then marks it valid. The running app is never replaced. Takes minutes (up to \
         45); returns the phase, build/health result and an error tail. Required before \
         `debug_report` may say `pass` when critical files changed."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Runs cargo and launches the built binary (isolated workspace).
        PermissionLevel::Execute
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        let (turn, task_id) = match turn_with_task("debug_validate_candidate") {
            Ok(v) => v,
            Err(refusal) => return Ok(refusal),
        };
        let root = turn.root.display().to_string();
        let rec = match candidate::start(&turn.ctx, Some(&root), Some(&task_id)).await {
            Ok(o) => o.value,
            Err(e) => return Ok(ToolResult::error(format!("cannot start candidate: {e}"))),
        };
        log::debug!(
            "[debug_mode] debug_validate_candidate task={task_id} candidate={}",
            rec.id
        );
        let mut guard = CancelOnDrop {
            ctx: Some(turn.ctx.clone()),
        };
        let rec = match candidate::wait(&turn.ctx, &rec.id, VALIDATE_WAIT, VALIDATE_POLL).await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("candidate wait failed: {e}"))),
        };
        guard.ctx = None;
        let mut out = candidate_json(&rec);
        if rec.phase.is_running() {
            out["note"] = json!(
                "still running after the wait limit; check it later with the \
                 debug_mode_candidate_status RPC or report `partial`"
            );
        }
        Ok(ToolResult::json(out))
    }
}

// ── debug_report ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportArgs {
    status: String,
    summary: String,
    #[serde(default)]
    validation: Vec<ReportCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportCheck {
    check: String,
    passed: bool,
    #[serde(default)]
    detail: Option<String>,
}

fn parse_status(raw: &str) -> Option<TaskStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "pass" => Some(TaskStatus::Pass),
        "partial" => Some(TaskStatus::Partial),
        "failed" => Some(TaskStatus::Failed),
        _ => None,
    }
}

/// Files this task changed so far: what the turn recorded plus the current
/// diff against the task's checkpoint.
async fn changed_files(turn: &DebugTurn, task_id: &str) -> Vec<String> {
    let mut files: Vec<String> = turn
        .ctx
        .store
        .task_get(task_id)
        .ok()
        .flatten()
        .map(|t| t.files_changed)
        .unwrap_or_default();
    let root = turn.root.display().to_string();
    if let Some(f) = turn::diff_files(&turn.ctx, &root, turn.checkpoint_id.as_deref()).await {
        files.extend(f);
    }
    files.sort();
    files.dedup();
    files
}

/// Self-modification gate (spec 35): records the task's critical files and
/// downgrades `pass` to `partial` when they changed without a validated
/// candidate for the current tree. Returns the (possibly new) status and a
/// message for the agent.
async fn self_mod_gate(
    turn: &DebugTurn,
    task_id: &str,
    files: &[String],
    status: TaskStatus,
) -> (TaskStatus, Option<String>) {
    let assessment = selfmod::assess(files);
    if !assessment.critical {
        return (status, None);
    }
    let crit = assessment.critical_files.clone();
    if let Err(e) = turn
        .ctx
        .store
        .task_modify(task_id, move |t| t.critical_files = crit)
    {
        log::warn!("[debug_mode] cannot record critical files: {e}");
    }
    if status != TaskStatus::Pass
        || candidate::candidate_valid_for_current_tree(&turn.ctx, &turn.root).await
    {
        return (status, None);
    }
    log::info!(
        "[debug_mode] debug_report pass downgraded task={task_id} critical_files={}",
        assessment.critical_files.len()
    );
    let shown: Vec<&str> = assessment
        .critical_files
        .iter()
        .take(8)
        .map(String::as_str)
        .collect();
    (
        TaskStatus::Partial,
        Some(format!(
            "DOWNGRADED from pass to partial: critical files changed ({}) and no validated \
             candidate matches the current tree. Run `debug_validate_candidate`, then call \
             `debug_report` again.",
            shown.join(", ")
        )),
    )
}

/// Final honest report for the current debug task.
pub struct DebugReportTool;

#[async_trait]
impl Tool for DebugReportTool {
    fn name(&self) -> &str {
        "debug_report"
    }

    fn description(&self) -> &str {
        "Debug mode only. Call once at the end of the turn to record the task outcome: \
         status (pass | partial | failed), a short summary of what changed, and the \
         validation checks you actually ran with their real results. `pass` is only \
         accepted when every listed check passed AND a check run through `debug_run_check` \
         passed after your last edit (otherwise it is recorded as `partial`). Checks you \
         list here are information only; they never count toward `pass`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["pass", "partial", "failed"] },
                "summary": { "type": "string", "description": "What changed and what is left." },
                "validation": {
                    "type": "array",
                    "description": "Checks that were actually run.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "check": { "type": "string", "description": "e.g. 'pnpm typecheck'" },
                            "passed": { "type": "boolean" },
                            "detail": { "type": "string", "description": "Short failure/skip note." }
                        },
                        "required": ["check", "passed"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["status", "summary"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Bookkeeping in the debug history only, like `todo` / `update_task`.
        PermissionLevel::None
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let (turn, task_id) = match turn_with_task("debug_report") {
            Ok(v) => v,
            Err(refusal) => return Ok(refusal),
        };
        let parsed: ReportArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        let Some(status) = parse_status(&parsed.status) else {
            return Ok(ToolResult::error(
                "`status` must be pass, partial or failed",
            ));
        };
        if parsed.summary.trim().is_empty() {
            return Ok(ToolResult::error("`summary` must not be empty"));
        }
        if status == TaskStatus::Pass && parsed.validation.iter().any(|c| !c.passed) {
            return Ok(ToolResult::error(
                "status `pass` is not allowed while a reported check failed; \
                 report `partial` or `failed`, or fix and re-run the check",
            ));
        }
        let files = changed_files(&turn, &task_id).await;
        let requested = status;
        let (mut status, downgrade) = self_mod_gate(&turn, &task_id, &files, status).await;
        let mut notes: Vec<String> = downgrade.into_iter().collect();
        // Real runs already persisted on the task (the report payload below is
        // self-attested and never counts toward the gate).
        let persisted: Vec<ValidationRecord> = turn
            .ctx
            .store
            .task_get(&task_id)
            .ok()
            .flatten()
            .map(|t| t.validation)
            .unwrap_or_default();
        if requested == TaskStatus::Pass {
            if let Some(note) = pass_gate::missing_check_note(&turn.root, &files, &persisted) {
                log::info!(
                    "[debug_mode] debug_report pass downgraded task={task_id} reason=no_post_edit_check files={}",
                    files.len()
                );
                status = TaskStatus::Partial;
                notes.push(note);
            }
        }
        let now = chrono::Utc::now().to_rfc3339();
        // Keep every real run, replace any earlier self-attested entries.
        let mut validation: Vec<ValidationRecord> = persisted
            .into_iter()
            .filter(pass_gate::is_real_check)
            .collect();
        let attested: Vec<ValidationRecord> = parsed
            .validation
            .into_iter()
            .take(MAX_VALIDATIONS)
            .map(|c| ValidationRecord {
                check_id: Some(c.check.chars().take(200).collect()),
                command: vec![],
                exit_code: None,
                passed: c.passed,
                timed_out: false,
                duration_ms: 0,
                at: now.clone(),
                output_tail: c.detail.unwrap_or_default(),
            })
            .collect();
        validation.extend(attested);
        let n_checks = validation.len();
        let patch = TaskPatch {
            status: Some(status),
            summary: Some(parsed.summary),
            validation: Some(validation),
            ..Default::default()
        };
        match ops::task_update(&turn.ctx, &task_id, patch).await {
            Ok(_) => {
                log::debug!(
                    "[debug_mode] debug_report task={task_id} status={status:?} checks={n_checks}"
                );
                let mut msg = format!(
                    "Recorded {status:?} for task {task_id} with {n_checks} validation check(s)."
                );
                for note in notes {
                    msg.push(' ');
                    msg.push_str(&note);
                }
                Ok(ToolResult::success(msg))
            }
            Err(e) => Ok(ToolResult::error(format!("report failed: {e}"))),
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
