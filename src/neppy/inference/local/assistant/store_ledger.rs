//! The effects ledger, step completion, boot recovery and retention: the half
//! of the state store that makes a resumed step safe.

use rusqlite::{params, OptionalExtension};

use super::store::{cap_summary, effect_from_row, StateStore};
use super::types::*;

/// A plan reduced to what is worth keeping after its task has finished: no
/// edit bodies, a few decisions, short text.
fn compact_plan(plan: &StepPlan) -> String {
    let compact = StepPlan {
        summary: clip(&plan.summary, 600),
        decisions: plan
            .decisions
            .iter()
            .take(3)
            .map(|d| clip(d, 120))
            .collect(),
        edits: plan
            .edits
            .iter()
            .map(|e| EditOp {
                path: clip(&e.path, 200),
                search: String::new(),
                replace: String::new(),
            })
            .collect(),
        run_tests: plan.run_tests,
        next_step: clip(&plan.next_step, 300),
        done: plan.done,
        search_queries: Vec::new(),
    };
    let json = serde_json::to_string(&compact).unwrap_or_default();
    if json.len() <= PLAN_JSON_DONE_MAX {
        return json;
    }
    let minimal = StepPlan {
        summary: clip(&plan.summary, 300),
        done: plan.done,
        run_tests: plan.run_tests,
        ..StepPlan::default()
    };
    clip(
        &serde_json::to_string(&minimal).unwrap_or_default(),
        PLAN_JSON_DONE_MAX,
    )
}

impl StateStore {
    // ---- effects -------------------------------------------------------

    /// Record that an effect is about to happen. A second call with the same
    /// key is a no-op, which is what makes the key unique per effect.
    pub fn record_effect_intent(&self, rec: &EffectRecord) -> Result<()> {
        self.conn.lock().execute(
            "INSERT OR IGNORE INTO effects(effect_key, task_id, step_no, kind, path,
                                           pre_sha, post_sha, status)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                rec.effect_key,
                rec.task_id,
                rec.step_no,
                rec.kind.as_str(),
                clip(&rec.path, 512),
                rec.pre_sha,
                rec.post_sha,
                rec.status.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn get_effect(&self, key: &str) -> Result<Option<EffectRecord>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT * FROM effects WHERE effect_key=?1",
                [key],
                effect_from_row,
            )
            .optional()?)
    }

    pub fn mark_effect(&self, key: &str, status: EffectStatus, result: Option<&str>) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE effects SET status=?2, result=COALESCE(?3, result) WHERE effect_key=?1",
            params![
                key,
                status.as_str(),
                result.map(|r| clip(r, EFFECT_RESULT_MAX))
            ],
        )?;
        Ok(())
    }

    pub fn effects_of_step(&self, task_id: &str, step_no: u32) -> Result<Vec<EffectRecord>> {
        let conn = self.conn.lock();
        let mut stmt =
            conn.prepare("SELECT * FROM effects WHERE task_id=?1 AND step_no=?2 ORDER BY rowid")?;
        let rows = stmt.query_map(params![task_id, step_no], effect_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- completion, boot, retention -----------------------------------

    /// Finish a step: its state, the task's summary, decisions, changed files,
    /// last test, next step and counters move together, or not at all.
    pub fn complete_step(&self, task_id: &str, step_no: u32, update: &TaskUpdate) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let (decisions_raw, changed_raw, done_before): (String, String, i64) = tx.query_row(
            "SELECT decisions, changed_files,
                    (SELECT count(*) FROM steps WHERE task_id=?1 AND step_no=?2 AND state='completed')
             FROM tasks WHERE id=?1",
            params![task_id, step_no],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut decisions: Vec<String> = serde_json::from_str(&decisions_raw).unwrap_or_default();
        for d in &update.new_decisions {
            let d = clip(d.trim(), DECISION_MAX_CHARS);
            if !d.is_empty() && !decisions.contains(&d) {
                decisions.push(d);
            }
        }
        if decisions.len() > DECISIONS_MAX {
            decisions.drain(..decisions.len() - DECISIONS_MAX);
        }
        let mut changed: Vec<String> = serde_json::from_str(&changed_raw).unwrap_or_default();
        for f in &update.new_changed_files {
            if !changed.contains(f) {
                changed.push(f.clone());
            }
        }
        if changed.len() > CHANGED_FILES_MAX {
            changed.drain(..changed.len() - CHANGED_FILES_MAX);
        }
        let last_test = update.last_test.as_ref().map(|t| {
            serde_json::to_string(&TestResult {
                tail: clip_tail(&t.tail, TEST_TAIL_STORE),
                ..t.clone()
            })
            .unwrap_or_default()
        });
        let now = now_ms();
        tx.execute(
            "UPDATE steps SET state='completed', completed_at=?3
             WHERE task_id=?1 AND step_no=?2",
            params![task_id, step_no, now],
        )?;
        tx.execute(
            "UPDATE effects SET status='done'
             WHERE task_id=?1 AND step_no=?2 AND status IN ('applied','intent')",
            params![task_id, step_no],
        )?;
        tx.execute(
            "UPDATE tasks SET summary=?2, decisions=?3, changed_files=?4,
                              last_test=COALESCE(?5, last_test), next_step=?6,
                              steps_done = steps_done + ?7, updated_at=?8
             WHERE id=?1",
            params![
                task_id,
                cap_summary(&update.summary),
                serde_json::to_string(&decisions)?,
                serde_json::to_string(&changed)?,
                last_test,
                clip(update.next_step.trim(), NEXT_STEP_MAX),
                i64::from(done_before == 0),
                now
            ],
        )?;
        if let Some(finish) = update.finish {
            tx.execute(
                "UPDATE tasks SET status=?2 WHERE id=?1",
                params![task_id, finish.as_str()],
            )?;
        }
        tx.commit()?;
        drop(conn);
        if let Some(finish) = update.finish {
            self.compact_finished_plans(task_id)?;
            log::info!(
                "[local_assistant:store] task {task_id} finished status={}",
                finish.as_str()
            );
        }
        log::debug!("[local_assistant:store] task {task_id} step {step_no} completed");
        Ok(())
    }

    /// Shrink the stored plans of a finished task to [`PLAN_JSON_DONE_MAX`].
    pub fn compact_finished_plans(&self, task_id: &str) -> Result<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let rows: Vec<(u32, String)> = {
            let mut stmt = tx.prepare(
                "SELECT step_no, plan_json FROM steps
                 WHERE task_id=?1 AND state='completed' AND plan_json IS NOT NULL
                   AND length(plan_json) > ?2",
            )?;
            let mapped = stmt.query_map(params![task_id, PLAN_JSON_DONE_MAX as i64], |r| {
                Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?))
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (no, json) in rows {
            let compact = serde_json::from_str::<StepPlan>(&json)
                .map(|p| compact_plan(&p))
                .unwrap_or_else(|_| clip(&json, PLAN_JSON_DONE_MAX));
            tx.execute(
                "UPDATE steps SET plan_json=?3 WHERE task_id=?1 AND step_no=?2",
                params![task_id, no, compact],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// At start-up nothing is running, whatever the database says: tasks left
    /// `running` or `queued` by a previous process become `interrupted`.
    pub fn mark_interrupted_on_boot(&self) -> Result<Vec<TaskId>> {
        let conn = self.conn.lock();
        let ids: Vec<String> = {
            let mut stmt = conn.prepare(
                "SELECT id FROM tasks WHERE status IN ('running','queued') ORDER BY created_at",
            )?;
            let mapped = stmt.query_map([], |r| r.get::<_, String>(0))?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        conn.execute(
            "UPDATE tasks SET status='interrupted', updated_at=?1
             WHERE status IN ('running','queued')",
            [now_ms()],
        )?;
        if !ids.is_empty() {
            log::info!(
                "[local_assistant:store] {} task(s) interrupted by a restart",
                ids.len()
            );
        }
        Ok(ids)
    }

    /// Finished tasks: keep the newest `keep_tasks`, and none older than
    /// `keep_days`. Unfinished tasks are never pruned. Returns rows removed.
    pub fn prune(&self, keep_tasks: usize, keep_days: u32, now: i64) -> Result<usize> {
        let cutoff = now - i64::from(keep_days) * 86_400_000;
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let doomed: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM tasks
                 WHERE status IN ('done','failed','budget_exhausted','cancelled')
                   AND (updated_at < ?1
                        OR id NOT IN (SELECT id FROM tasks
                                      WHERE status IN ('done','failed','budget_exhausted','cancelled')
                                      ORDER BY updated_at DESC, rowid DESC LIMIT ?2))",
            )?;
            let mapped = stmt.query_map(params![cutoff, keep_tasks as i64], |r| {
                r.get::<_, String>(0)
            })?;
            mapped.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for id in &doomed {
            tx.execute("DELETE FROM effects WHERE task_id=?1", [id])?;
            tx.execute("DELETE FROM steps WHERE task_id=?1", [id])?;
            tx.execute("DELETE FROM tasks WHERE id=?1", [id])?;
        }
        tx.commit()?;
        if !doomed.is_empty() {
            log::debug!(
                "[local_assistant:store] pruned {} finished task(s)",
                doomed.len()
            );
        }
        Ok(doomed.len())
    }

    #[cfg(test)]
    pub(crate) fn count(&self, table: &str) -> i64 {
        self.conn
            .lock()
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    }

    #[cfg(test)]
    pub(crate) fn backdate(&self, id: &str, updated_at: i64) {
        self.conn
            .lock()
            .execute(
                "UPDATE tasks SET updated_at=?2 WHERE id=?1",
                params![id, updated_at],
            )
            .unwrap();
    }
}
