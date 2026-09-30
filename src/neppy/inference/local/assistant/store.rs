//! Durable task state: tasks, steps and the effects ledger.
//!
//! Every transition is transactional, and a step advances only after the work
//! its new state names is durable. That is what lets a killed process resume
//! without repeating anything: the plan is stored before any side effect, each
//! side effect has a unique key and a recorded before/after hash, and a step's
//! completion and the task's updated summary commit together.
//!
//! tinyagents' run ledger was considered and not used: it cannot enforce a
//! unique effect key, lives in a vendored crate, and its orphan handling would
//! fight this resume.

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::types::*;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    project_root TEXT NOT NULL,
    goal TEXT NOT NULL,
    status TEXT NOT NULL,
    allow_edits INTEGER NOT NULL,
    test_command TEXT,
    max_steps INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    decisions TEXT NOT NULL DEFAULT '[]',
    changed_files TEXT NOT NULL DEFAULT '[]',
    last_test TEXT,
    next_step TEXT NOT NULL DEFAULT '',
    completion_tokens_used INTEGER NOT NULL DEFAULT 0,
    steps_done INTEGER NOT NULL DEFAULT 0,
    error TEXT
);
CREATE TABLE IF NOT EXISTS steps (
    task_id TEXT NOT NULL,
    step_no INTEGER NOT NULL,
    state TEXT NOT NULL,
    plan_json TEXT,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    completion_tokens INTEGER NOT NULL DEFAULT 0,
    started_at INTEGER NOT NULL,
    completed_at INTEGER,
    PRIMARY KEY (task_id, step_no)
);
CREATE TABLE IF NOT EXISTS effects (
    effect_key TEXT PRIMARY KEY,
    task_id TEXT NOT NULL,
    step_no INTEGER NOT NULL,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    pre_sha TEXT NOT NULL,
    post_sha TEXT NOT NULL,
    status TEXT NOT NULL,
    result TEXT
);
CREATE INDEX IF NOT EXISTS effects_task ON effects(task_id, step_no);
CREATE INDEX IF NOT EXISTS tasks_status ON tasks(status, updated_at);
";

pub struct StateStore {
    pub(super) conn: Mutex<Connection>,
}

/// Drop the oldest sentences until `summary` fits [`SUMMARY_MAX`]. Newer
/// material is kept because the model rewrites the summary each step and its
/// most recent additions are the ones not yet folded in.
pub fn cap_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.len() <= SUMMARY_MAX {
        return trimmed.to_string();
    }
    let mut out = trimmed;
    while out.len() > SUMMARY_MAX {
        match out.find(['.', '\n']) {
            Some(at) if at + 1 < out.len() => out = out[at + 1..].trim_start(),
            _ => return clip_tail(out, SUMMARY_MAX),
        }
    }
    out.to_string()
}

fn task_from_row(row: &Row<'_>) -> rusqlite::Result<TaskRecord> {
    let status: String = row.get("status")?;
    let decisions: String = row.get("decisions")?;
    let changed: String = row.get("changed_files")?;
    let last_test: Option<String> = row.get("last_test")?;
    Ok(TaskRecord {
        id: row.get("id")?,
        project_root: row.get("project_root")?,
        goal: row.get("goal")?,
        status: TaskStatus::parse(&status).unwrap_or(TaskStatus::Failed),
        allow_edits: row.get::<_, i64>("allow_edits")? != 0,
        test_command: row.get("test_command")?,
        max_steps: row.get::<_, i64>("max_steps")? as u32,
        created_at_ms: row.get("created_at")?,
        updated_at_ms: row.get("updated_at")?,
        summary: row.get("summary")?,
        decisions: serde_json::from_str(&decisions).unwrap_or_default(),
        changed_files: serde_json::from_str(&changed).unwrap_or_default(),
        last_test: last_test.and_then(|t| serde_json::from_str(&t).ok()),
        next_step: row.get("next_step")?,
        completion_tokens_used: row.get::<_, i64>("completion_tokens_used")? as u64,
        steps_done: row.get::<_, i64>("steps_done")? as u32,
        error: row.get("error")?,
    })
}

pub(super) fn effect_from_row(row: &Row<'_>) -> rusqlite::Result<EffectRecord> {
    let kind: String = row.get("kind")?;
    let status: String = row.get("status")?;
    Ok(EffectRecord {
        effect_key: row.get("effect_key")?,
        task_id: row.get("task_id")?,
        step_no: row.get::<_, i64>("step_no")? as u32,
        kind: if kind == "test" {
            EffectKind::Test
        } else {
            EffectKind::Edit
        },
        path: row.get("path")?,
        pre_sha: row.get("pre_sha")?,
        post_sha: row.get("post_sha")?,
        status: EffectStatus::parse(&status).unwrap_or(EffectStatus::Conflict),
        result: row.get("result")?,
    })
}

impl StateStore {
    /// `{workspace}/local_assistant/state.db`.
    pub fn open(workspace: &Path) -> Result<Self> {
        Self::open_path(&workspace.join("local_assistant").join("state.db"))
    }

    pub fn open_path(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        // FULL: a checkpoint that is not on disk is not a checkpoint.
        conn.execute_batch("PRAGMA synchronous=FULL;")?;
        conn.execute_batch(SCHEMA)?;
        log::debug!("[local_assistant:store] opened {}", path.display());
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn create_task(&self, spec: &TaskSpec, root: &str, max_steps: u32) -> Result<TaskRecord> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let now = now_ms();
        self.conn.lock().execute(
            "INSERT INTO tasks(id, project_root, goal, status, allow_edits, test_command,
                               max_steps, created_at, updated_at)
             VALUES(?1, ?2, ?3, 'queued', ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                root,
                clip(spec.goal.trim(), 4000),
                i64::from(spec.allow_edits),
                spec.test_command.as_deref().map(|c| clip(c, 2000)),
                max_steps,
                now
            ],
        )?;
        log::info!("[local_assistant:store] task {id} created status=queued");
        self.require_task(&id)
    }

    pub fn load_task(&self, id: &str) -> Result<Option<TaskRecord>> {
        Ok(self
            .conn
            .lock()
            .query_row("SELECT * FROM tasks WHERE id=?1", [id], task_from_row)
            .optional()?)
    }

    pub fn require_task(&self, id: &str) -> Result<TaskRecord> {
        self.load_task(id)?
            .ok_or_else(|| AssistantError::NotFound(id.to_string()))
    }

    /// Newest first.
    pub fn list_tasks(&self, limit: usize) -> Result<Vec<TaskRecord>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT * FROM tasks ORDER BY created_at DESC, rowid DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], task_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_status(&self, id: &str, status: TaskStatus, error: Option<&str>) -> Result<()> {
        let changed = self.conn.lock().execute(
            "UPDATE tasks SET status=?2, error=?3, updated_at=?4 WHERE id=?1",
            params![
                id,
                status.as_str(),
                error.map(|e| clip(e, ERROR_MAX)),
                now_ms()
            ],
        )?;
        if changed == 0 {
            return Err(AssistantError::NotFound(id.to_string()));
        }
        log::info!(
            "[local_assistant:store] task {id} status={}",
            status.as_str()
        );
        Ok(())
    }

    /// Move to `status` only if the task is currently in one of `from`.
    /// Returns whether it moved. Guards a cancel against a finishing task.
    pub fn transition(&self, id: &str, from: &[TaskStatus], to: TaskStatus) -> Result<bool> {
        let conn = self.conn.lock();
        let current: Option<String> = conn
            .query_row("SELECT status FROM tasks WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        let Some(current) = current.and_then(|c| TaskStatus::parse(&c)) else {
            return Err(AssistantError::NotFound(id.to_string()));
        };
        if !from.contains(&current) {
            return Ok(false);
        }
        conn.execute(
            "UPDATE tasks SET status=?2, updated_at=?3 WHERE id=?1",
            params![id, to.as_str(), now_ms()],
        )?;
        log::info!(
            "[local_assistant:store] task {id} {} -> {}",
            current.as_str(),
            to.as_str()
        );
        Ok(true)
    }

    // ---- steps ---------------------------------------------------------

    pub fn begin_step(&self, task_id: &str, step_no: u32) -> Result<()> {
        self.conn.lock().execute(
            "INSERT OR IGNORE INTO steps(task_id, step_no, state, started_at)
             VALUES(?1, ?2, 'started', ?3)",
            params![task_id, step_no, now_ms()],
        )?;
        log::debug!("[local_assistant:store] task {task_id} step {step_no} begun");
        Ok(())
    }

    pub fn load_step(&self, task_id: &str, step_no: u32) -> Result<Option<StepRecord>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT state, plan_json, prompt_tokens, completion_tokens
                 FROM steps WHERE task_id=?1 AND step_no=?2",
                params![task_id, step_no],
                |r| {
                    let state: String = r.get(0)?;
                    let plan: Option<String> = r.get(1)?;
                    Ok(StepRecord {
                        task_id: task_id.to_string(),
                        step_no,
                        state: StepState::parse(&state).unwrap_or(StepState::Started),
                        plan: plan.and_then(|p| serde_json::from_str(&p).ok()),
                        prompt_tokens: r.get::<_, i64>(2)? as u64,
                        completion_tokens: r.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .optional()?)
    }

    /// The highest-numbered step, finished or not.
    pub fn latest_step(&self, task_id: &str) -> Result<Option<StepRecord>> {
        let no: Option<i64> = self.conn.lock().query_row(
            "SELECT max(step_no) FROM steps WHERE task_id=?1",
            [task_id],
            |r| r.get(0),
        )?;
        match no {
            Some(no) => self.load_step(task_id, no as u32),
            None => Ok(None),
        }
    }

    /// Persist the model's plan, before any side effect. Counts the step's
    /// completion tokens against the task exactly once: a replayed step that
    /// is already `planned` adds nothing.
    pub fn save_plan(
        &self,
        task_id: &str,
        step_no: u32,
        plan: &StepPlan,
        prompt_tokens: u64,
        completion_tokens: u64,
    ) -> Result<()> {
        let json = serde_json::to_string(plan)?;
        if json.len() > PLAN_JSON_MAX {
            return Err(AssistantError::Invalid(format!(
                "plan is {} bytes; the limit is {PLAN_JSON_MAX}",
                json.len()
            )));
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let moved = tx.execute(
            "UPDATE steps SET state='planned', plan_json=?3, prompt_tokens=?4, completion_tokens=?5
             WHERE task_id=?1 AND step_no=?2 AND state='started'",
            params![
                task_id,
                step_no,
                json,
                prompt_tokens as i64,
                completion_tokens as i64
            ],
        )?;
        if moved > 0 {
            tx.execute(
                "UPDATE tasks SET completion_tokens_used = completion_tokens_used + ?2,
                                  updated_at = ?3 WHERE id=?1",
                params![task_id, completion_tokens as i64, now_ms()],
            )?;
        }
        tx.commit()?;
        log::debug!(
            "[local_assistant:store] task {task_id} step {step_no} planned \
             (edits={} prompt~{prompt_tokens} completion={completion_tokens})",
            plan.edits.len()
        );
        Ok(())
    }

    /// Count completion tokens spent on a step that produced no plan.
    pub fn add_tokens(&self, task_id: &str, tokens: u64) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE tasks SET completion_tokens_used = completion_tokens_used + ?2 WHERE id=?1",
            params![task_id, tokens as i64],
        )?;
        Ok(())
    }

    /// Move a step forward. Never backwards.
    pub fn set_step_state(&self, task_id: &str, step_no: u32, state: StepState) -> Result<()> {
        let current = self.load_step(task_id, step_no)?.map(|s| s.state);
        if current.is_some_and(|c| c >= state) {
            return Ok(());
        }
        self.conn.lock().execute(
            "UPDATE steps SET state=?3 WHERE task_id=?1 AND step_no=?2",
            params![task_id, step_no, state.as_str()],
        )?;
        log::debug!(
            "[local_assistant:store] task {task_id} step {step_no} state={}",
            state.as_str()
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
