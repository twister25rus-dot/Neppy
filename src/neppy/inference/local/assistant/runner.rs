//! The step loop: one task, one step at a time, every transition durable.
//!
//! A step is a short state machine (`started -> planned -> applying ->
//! applied -> tested -> completed`). The model is called in exactly one state,
//! `started`; its reply is stored before anything acts on it, so a resumed
//! step replays the stored plan instead of asking again. Edits and the test run
//! go through the effects ledger, so they happen once however many times the
//! step is entered. Completing a step updates the task record in the same
//! transaction.
//!
//! Nothing accumulates between steps: the next prompt is built from the task
//! record and fresh retrieval, never from earlier replies.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::neppy::config::schema::LocalAssistantConfig;
use crate::neppy::security::SecurityPolicy;

use super::super::service::mlx_admin::metrics::MetricsSink;
use super::faults::{check, FaultPoint, Faults};
use super::index::{open_index, ProjectIndex};
use super::model::StepModel;
use super::planning::{plan_step, PlanStep};
use super::prompt::TokenEstimator;
use super::step_effects::{apply_step_edits, run_step_tests, stored_test_result, TestsStep};
use super::store::StateStore;
use super::tests_runner::StopReason;
use super::types::*;

/// Edit problems reported back to the model per step.
const NOTES_PER_STEP: usize = 5;

/// Everything `run_task` needs, injected so tests can script the model, the
/// policy and crashes.
pub(crate) struct RunEnv {
    pub(crate) store: Arc<StateStore>,
    pub(crate) workspace: PathBuf,
    pub(crate) cfg: LocalAssistantConfig,
    pub(crate) policy: Arc<SecurityPolicy>,
    pub(crate) model: Arc<dyn StepModel>,
    pub(crate) fallback: Option<Arc<dyn StepModel>>,
    pub(crate) faults: Arc<dyn Faults>,
    pub(crate) metrics: Option<Arc<MetricsSink>>,
    /// Base delay before an in-place model retry. Multiplied by the attempt.
    pub(crate) retry_delay: Duration,
}

impl RunEnv {
    pub(super) fn event(&self, kind: &str, detail: impl Into<String>) {
        if let Some(metrics) = &self.metrics {
            metrics.event(kind, None, detail);
        }
    }
}

/// How a run is asked to stop.
#[derive(Clone)]
pub(crate) struct StopSignal {
    /// The master switch; checked between steps.
    pub(crate) enabled: Arc<AtomicBool>,
    /// Cancels the task, aborting an in-flight model call.
    pub(crate) cancel: CancellationToken,
}

impl StopSignal {
    pub(crate) fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(true)),
            cancel: CancellationToken::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunOutcome {
    /// The task reached a terminal status.
    Finished(TaskStatus),
    /// Stopped between steps because the assistant was disabled.
    Paused,
    /// A model call was refused, paused or preempted (memory pressure). The
    /// step's durable state is intact; run the task again after resume.
    Preempted(String),
}

enum StepResult {
    Completed,
    Preempted(String),
    Cancelled,
    /// The assistant was disabled while the step's tests were running.
    Paused,
    BudgetExhausted,
}

/// Run `task_id` until it finishes, is paused, is preempted, or is cancelled.
///
/// An error other than a simulated crash fails the task (recorded, resumable);
/// a simulated crash (`AssistantError::Interrupted` from the fault injector)
/// is returned as is with the task left `running`, as a killed process would.
pub(crate) async fn run_task(env: &RunEnv, task_id: &str, stop: &StopSignal) -> Result<RunOutcome> {
    let task = env.store.require_task(task_id)?;
    if task.status.is_terminal() {
        return Ok(RunOutcome::Finished(task.status));
    }
    match drive(env, task, stop).await {
        Ok(outcome) => Ok(outcome),
        Err(AssistantError::Interrupted(msg)) => Err(AssistantError::Interrupted(msg)),
        Err(err) => {
            log::warn!("[local_assistant] task {task_id} failed: {err}");
            env.store
                .set_status(task_id, TaskStatus::Failed, Some(&err.to_string()))?;
            env.event(
                "task_checkpoint",
                format!("task {task_id} failed; checkpoint kept"),
            );
            Ok(RunOutcome::Finished(TaskStatus::Failed))
        }
    }
}

async fn drive(env: &RunEnv, task: TaskRecord, stop: &StopSignal) -> Result<RunOutcome> {
    let resumed = matches!(task.status, TaskStatus::Interrupted | TaskStatus::Paused);
    env.store.set_status(&task.id, TaskStatus::Running, None)?;
    if resumed {
        log::info!(
            "[local_assistant] task {} resumed at step {} ({} done)",
            task.id,
            task.steps_done + 1,
            task.steps_done
        );
        env.event(
            "task_resumed",
            format!("task {} steps_done={}", task.id, task.steps_done),
        );
    }
    let root = Path::new(&task.project_root)
        .canonicalize()
        .map_err(|e| AssistantError::Invalid(format!("project root is not available: {e}")))?;
    let index = Arc::new(open_index(&env.workspace, &root)?);
    let edits_allowed = task.allow_edits && env.policy.can_act();
    // The root was checked when the task was accepted; the config, the tree and
    // the tier can all have changed since. Same rules, applied again.
    super::guards::validate_root(&env.policy, &env.workspace, &root, edits_allowed).map_err(
        |why| {
            log::warn!(
                "[local_assistant] task {} root no longer allowed: {why}",
                task.id
            );
            AssistantError::Invalid(format!("project root is no longer allowed: {why}"))
        },
    )?;
    let mut estimator = TokenEstimator::default();
    let result = step_loop(
        env,
        &index,
        &root,
        &task.id,
        edits_allowed,
        &mut estimator,
        stop,
    )
    .await;
    // Release what a request held: the cache of retrieved text.
    index.clear_cache();
    drop(index);
    if let Ok(RunOutcome::Finished(_)) = &result {
        let _ = env
            .store
            .prune(env.cfg.keep_tasks, env.cfg.keep_days, now_ms());
    }
    result
}

fn finish_cancelled(env: &RunEnv, id: &str) -> Result<RunOutcome> {
    env.store.transition(
        id,
        &[
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Interrupted,
        ],
        TaskStatus::Cancelled,
    )?;
    log::info!("[local_assistant] task {id} cancelled");
    env.event("task_checkpoint", format!("task {id} cancelled"));
    Ok(RunOutcome::Finished(TaskStatus::Cancelled))
}

fn pause_disabled(env: &RunEnv, id: &str) -> Result<RunOutcome> {
    env.store.set_status(id, TaskStatus::Paused, None)?;
    log::info!("[local_assistant] task {id} paused: the assistant is disabled");
    env.event(
        "task_checkpoint",
        format!("task {id} paused at a step boundary"),
    );
    Ok(RunOutcome::Paused)
}

fn budget_exhausted(env: &RunEnv, id: &str, why: &str) -> Result<RunOutcome> {
    env.store
        .set_status(id, TaskStatus::BudgetExhausted, Some(why))?;
    log::info!("[local_assistant] task {id} budget exhausted: {why}");
    env.event(
        "task_checkpoint",
        format!("task {id} budget_exhausted: {why}"),
    );
    Ok(RunOutcome::Finished(TaskStatus::BudgetExhausted))
}

async fn step_loop(
    env: &RunEnv,
    index: &Arc<ProjectIndex>,
    root: &Path,
    id: &str,
    edits_allowed: bool,
    estimator: &mut TokenEstimator,
    stop: &StopSignal,
) -> Result<RunOutcome> {
    loop {
        if stop.cancel.is_cancelled() {
            return finish_cancelled(env, id);
        }
        if !stop.enabled.load(Ordering::SeqCst) {
            return pause_disabled(env, id);
        }
        let task = env.store.require_task(id)?;
        if task.status.is_terminal() {
            return Ok(RunOutcome::Finished(task.status));
        }
        let latest = env.store.latest_step(id)?;
        let (step_no, fresh) = match &latest {
            Some(s) if s.state != StepState::Completed => (s.step_no, false),
            Some(s) => (s.step_no + 1, true),
            None => (1, true),
        };
        // A step already planned has been paid for; only a new one is budgeted.
        if fresh {
            if task.steps_done >= task.max_steps {
                return budget_exhausted(env, id, &format!("{} steps used", task.steps_done));
            }
            if task.completion_tokens_used >= u64::from(env.cfg.task_max_completion_tokens) {
                return budget_exhausted(
                    env,
                    id,
                    &format!("{} completion tokens used", task.completion_tokens_used),
                );
            }
        }
        match run_step(
            env,
            index,
            root,
            &task,
            step_no,
            edits_allowed,
            estimator,
            stop,
        )
        .await?
        {
            StepResult::Completed => {}
            StepResult::Cancelled => return finish_cancelled(env, id),
            StepResult::Paused => return pause_disabled(env, id),
            StepResult::BudgetExhausted => {
                return budget_exhausted(env, id, "completion token budget reached mid-step");
            }
            StepResult::Preempted(why) => {
                env.store.set_status(id, TaskStatus::Paused, Some(&why))?;
                log::info!("[local_assistant] task {id} step {step_no} preempted: {why}");
                env.event(
                    "task_checkpoint",
                    format!("task {id} step {step_no} preempted: {why}"),
                );
                return Ok(RunOutcome::Preempted(why));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_step(
    env: &RunEnv,
    index: &Arc<ProjectIndex>,
    root: &Path,
    task: &TaskRecord,
    step_no: u32,
    edits_allowed: bool,
    estimator: &mut TokenEstimator,
    stop: &StopSignal,
) -> Result<StepResult> {
    let store = &env.store;
    store.begin_step(&task.id, step_no)?;
    let mut step = store
        .load_step(&task.id, step_no)?
        .ok_or_else(|| AssistantError::Storage("step row missing after begin".into()))?;
    log::info!(
        "[local_assistant] task {} step {step_no} entering state={}",
        task.id,
        step.state.as_str()
    );

    if step.state == StepState::Started {
        match plan_step(env, index, task, step_no, edits_allowed, estimator, stop).await? {
            PlanStep::Planned => {}
            PlanStep::Preempted(why) => return Ok(StepResult::Preempted(why)),
            PlanStep::Cancelled => return Ok(StepResult::Cancelled),
            PlanStep::BudgetExhausted => return Ok(StepResult::BudgetExhausted),
        }
        check(env.faults.as_ref(), FaultPoint::Planned)?;
        step = store
            .load_step(&task.id, step_no)?
            .ok_or_else(|| AssistantError::Storage("step row vanished".into()))?;
    }
    let plan = step
        .plan
        .clone()
        .ok_or_else(|| AssistantError::Storage("the stored plan is unreadable".into()))?;

    if step.state < StepState::Applied {
        apply_step_edits(env, root, task, step_no, &plan, edits_allowed)?;
        store.set_step_state(&task.id, step_no, StepState::Applied)?;
        check(env.faults.as_ref(), FaultPoint::Applied)?;
    }

    let test = if step.state < StepState::Tested {
        let result = match run_step_tests(env, root, task, step_no, &plan, stop).await? {
            TestsStep::Result(result) => result,
            TestsStep::Stopped(StopReason::Cancelled) => return Ok(StepResult::Cancelled),
            TestsStep::Stopped(StopReason::Disabled) => return Ok(StepResult::Paused),
        };
        store.set_step_state(&task.id, step_no, StepState::Tested)?;
        check(env.faults.as_ref(), FaultPoint::Tested)?;
        result
    } else {
        stored_test_result(store, &task.id, step_no)?
    };

    let effects = store.effects_of_step(&task.id, step_no)?;
    let mut changed: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    for e in effects.iter().filter(|e| e.kind == EffectKind::Edit) {
        match e.status {
            EffectStatus::Applied | EffectStatus::Done => {
                if !changed.contains(&e.path) {
                    changed.push(e.path.clone());
                }
            }
            EffectStatus::Conflict | EffectStatus::Refused if notes.len() < NOTES_PER_STEP => {
                notes.push(format!(
                    "edit on {} was not applied ({}): {}",
                    e.path,
                    e.status.as_str(),
                    e.result.clone().unwrap_or_default()
                ));
            }
            _ => {}
        }
    }
    let test_failed = test
        .as_ref()
        .is_some_and(|t| (t.exit_code != 0 && t.exit_code != TEST_REFUSED) || t.timed_out);
    let mut decisions = plan.decisions.clone();
    decisions.extend(notes);
    let update = TaskUpdate {
        summary: if plan.summary.trim().is_empty() {
            task.summary.clone()
        } else {
            plan.summary.clone()
        },
        new_decisions: decisions,
        new_changed_files: changed.clone(),
        last_test: test.clone(),
        next_step: plan.next_step.clone(),
        finish: (plan.done && !test_failed).then_some(TaskStatus::Done),
    };
    store.complete_step(&task.id, step_no, &update)?;
    log::info!(
        "[local_assistant] task {} step {step_no} completed (changed={} test={} done={})",
        task.id,
        changed.len(),
        test.as_ref()
            .map_or("none".to_string(), |t| t.exit_code.to_string()),
        update.finish.is_some()
    );
    env.event(
        "task_step_completed",
        format!(
            "task {} step {step_no} prompt_tokens={} completion_tokens={} context_limit={} changed={}",
            task.id,
            step.prompt_tokens,
            step.completion_tokens,
            env.cfg.context_limit_tokens,
            changed.len()
        ),
    );
    Ok(StepResult::Completed)
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runner_review_tests.rs"]
mod review_tests;
