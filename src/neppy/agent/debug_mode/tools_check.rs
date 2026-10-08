//! `debug_run_check`: run a project check for the current debug task and
//! persist the result as a real [`super::types::ValidationRecord`].
//!
//! Only checks run through this tool count toward `pass`
//! (see [`super::pass_gate`]); it reuses `ops::run_check`, so it can run exactly
//! the commands Debug Mode already allows (`checks::validate_command`).

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::neppy::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::checks;
use super::ops;
use super::pass_gate;
use super::secrets;
use super::tools::turn_with_task;

/// Chars of combined stdout/stderr returned to the model.
const RESULT_TAIL_CHARS: usize = 3000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunCheckArgs {
    #[serde(default)]
    check_id: Option<String>,
    #[serde(default)]
    command: Option<Vec<String>>,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// Last `max` chars of `s`.
fn tail_chars(s: &str, max: usize) -> String {
    let skip = s.chars().count().saturating_sub(max);
    s.chars().skip(skip).collect()
}

/// Runs a validation command and records it on the current debug task.
pub struct DebugRunCheckTool;

#[async_trait]
impl Tool for DebugRunCheckTool {
    fn name(&self) -> &str {
        "debug_run_check"
    }

    fn description(&self) -> &str {
        "Debug mode only. Run a validation check (tests, typecheck, lint, build) for the \
         current debug task and record the real result (command, exit code, time, output \
         tail) on it. Use this after editing, for every check you rely on: only checks run \
         through this tool count toward `pass` in `debug_report`, and a `pass` needs a \
         passing TEST, TYPECHECK, LINT or BUILD check (cargo test/check/clippy/build, \
         package.json test/typecheck/lint/build/compile/check scripts) that ran AFTER your \
         last edit. Commands that verify nothing (git status/diff/rev-parse, cargo \
         tree/metadata/fmt, format checks) run fine but never count. Give either `check_id` (from the \
         discovered checks) or `command` as an argv array; only commands Debug mode already \
         allows can run. Returns passed, exit_code, duration and the output tail."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "check_id": {
                    "type": "string",
                    "description": "Id of a discovered check (see the project's checks). Use this OR `command`."
                },
                "command": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command as an argv array, e.g. [\"cargo\", \"test\", \"--lib\"]. Use this OR `check_id`."
                },
                "timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional timeout in seconds (default 600, max 3600)."
                }
            },
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Runs project build/test tooling, like `run_tests` / `run_linter`.
        PermissionLevel::Execute
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let (turn, task_id) = match turn_with_task("debug_run_check") {
            Ok(v) => v,
            Err(refusal) => return Ok(refusal),
        };
        let parsed: RunCheckArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return Ok(ToolResult::error(format!("invalid arguments: {e}"))),
        };
        log::debug!(
            "[debug_mode] debug_run_check task={task_id} check_id={} argv_len={}",
            parsed.check_id.as_deref().unwrap_or("-"),
            parsed.command.as_ref().map_or(0, Vec::len)
        );
        let root = turn.root.display().to_string();
        let res = ops::run_check(
            &turn.ctx,
            Some(&root),
            parsed.check_id.as_deref(),
            parsed.command,
            parsed.timeout_secs,
            Some(&task_id),
        )
        .await;
        match res {
            Ok(o) => {
                let r = o.value;
                log::info!(
                    "[debug_mode] debug_run_check done task={task_id} passed={} exit={:?} \
                     timed_out={} duration_ms={}",
                    r.passed,
                    r.exit_code,
                    r.timed_out,
                    r.duration_ms
                );
                let discovered = checks::discover(&turn.root);
                let is_verification = pass_gate::is_verification(&r.command, &discovered);
                let counts_for_pass = is_verification && r.passed && !r.timed_out;
                log::debug!(
                    "[debug_mode] debug_run_check task={task_id} is_verification={is_verification} \
                     counts_for_pass={counts_for_pass}"
                );
                let combined = format!("{}\n{}", r.stdout_tail, r.stderr_tail);
                let tail = secrets::mask_secret_like_lines(&tail_chars(
                    combined.trim(),
                    RESULT_TAIL_CHARS,
                ));
                Ok(ToolResult::json(json!({
                    "passed": r.passed,
                    "is_verification": is_verification,
                    "counts_for_pass": counts_for_pass,
                    "exit_code": r.exit_code,
                    "timed_out": r.timed_out,
                    "duration_ms": r.duration_ms,
                    "check_id": r.check_id,
                    "command": r.command,
                    "recorded_on_task": task_id,
                    "output_tail": tail,
                })))
            }
            Err(e) => {
                log::info!("[debug_mode] debug_run_check refused task={task_id}: {e}");
                Ok(ToolResult::error(format!("check not run: {e}")))
            }
        }
    }
}

#[cfg(test)]
#[path = "tools_check_tests.rs"]
mod tests;
