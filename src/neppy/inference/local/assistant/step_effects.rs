//! The side-effecting half of a step: applying edits and running the test
//! command, both through the effects ledger so each happens once.

use std::path::Path;
use std::time::Duration;

use super::command_policy::authorize_run;
use super::edits::{apply_edit, refuse_disabled, test_key, EditCtx};
use super::faults::{check, FaultPoint};
use super::runner::{RunEnv, StopSignal};
use super::store::StateStore;
use super::tests_runner::{refused_result, run_test_command_until, StopReason, TestOutcome};
use super::types::*;

/// Tail of a test result kept in the effects ledger, bytes.
const EFFECT_TAIL: usize = 1800;

pub(super) fn stored_test_result(
    store: &StateStore,
    task_id: &str,
    step_no: u32,
) -> Result<Option<TestResult>> {
    Ok(store
        .get_effect(&test_key(task_id, step_no))?
        .and_then(|e| e.result)
        .and_then(|r| serde_json::from_str(&r).ok()))
}

pub(super) fn apply_step_edits(
    env: &RunEnv,
    root: &Path,
    task: &TaskRecord,
    step_no: u32,
    plan: &StepPlan,
    edits_allowed: bool,
) -> Result<()> {
    if plan.edits.is_empty() {
        return Ok(());
    }
    env.store
        .set_step_state(&task.id, step_no, StepState::Applying)?;
    let ctx = EditCtx {
        root,
        policy: &env.policy,
        workspace: &env.workspace,
        cfg: &env.cfg,
        store: &env.store,
        task_id: &task.id,
        step_no,
        faults: env.faults.as_ref(),
    };
    for (idx, edit) in plan.edits.iter().enumerate() {
        let outcome = if edits_allowed {
            apply_edit(&ctx, idx, edit)?
        } else {
            let why = if task.allow_edits {
                "the autonomy tier does not allow edits"
            } else {
                "edits are not enabled for this task"
            };
            refuse_disabled(&ctx, idx, edit, why)?
        };
        log::debug!(
            "[local_assistant] task {} step {step_no} edit {idx} -> {}",
            task.id,
            match &outcome {
                EditOutcome::Applied => "applied",
                EditOutcome::AlreadyApplied => "already applied",
                EditOutcome::Conflict(_) => "conflict",
                EditOutcome::Refused(_) => "refused",
            }
        );
    }
    Ok(())
}

/// What running a step's tests came to.
pub(super) enum TestsStep {
    /// The tests ran (or were not wanted); the result, if any.
    Result(Option<TestResult>),
    /// The command was killed because the task was cancelled or the assistant
    /// disabled. Its ledger row is left open, so a resume runs it again.
    Stopped(StopReason),
}

pub(super) async fn run_step_tests(
    env: &RunEnv,
    root: &Path,
    task: &TaskRecord,
    step_no: u32,
    plan: &StepPlan,
    stop: &StopSignal,
) -> Result<TestsStep> {
    if !plan.run_tests {
        return Ok(TestsStep::Result(None));
    }
    let Some(command) = &task.test_command else {
        return Ok(TestsStep::Result(None));
    };
    let key = test_key(&task.id, step_no);
    if let Some(done) = env
        .store
        .get_effect(&key)?
        .filter(|e| e.status == EffectStatus::Done)
    {
        // Ran before the crash; its result is the result.
        log::info!(
            "[local_assistant] task {} step {step_no} test already ran; reusing its result",
            task.id
        );
        return Ok(TestsStep::Result(
            done.result.and_then(|r| serde_json::from_str(&r).ok()),
        ));
    }
    env.store.record_effect_intent(&EffectRecord {
        effect_key: key.clone(),
        task_id: task.id.clone(),
        step_no,
        kind: EffectKind::Test,
        path: String::new(),
        pre_sha: String::new(),
        post_sha: String::new(),
        status: EffectStatus::Intent,
        result: None,
    })?;
    check(env.faults.as_ref(), FaultPoint::TestIntent)?;
    // Re-checked on every run, not only when the task was accepted: the policy
    // can change while a task waits, and the action budget is charged here.
    let result = match authorize_run(&env.policy, command, root) {
        Err(why) => {
            log::warn!(
                "[local_assistant] task {} test command refused: {why}",
                task.id
            );
            refused_result(&why)
        }
        Ok(()) => {
            match run_test_command_until(
                root,
                command,
                Duration::from_secs(env.cfg.test_timeout_secs.max(1)),
                stop,
            )
            .await
            {
                TestOutcome::Finished(result) => result,
                TestOutcome::Stopped(reason) => return Ok(TestsStep::Stopped(reason)),
            }
        }
    };
    let stored = TestResult {
        tail: clip_tail(&result.tail, EFFECT_TAIL),
        ..result.clone()
    };
    env.store.mark_effect(
        &key,
        EffectStatus::Done,
        Some(&serde_json::to_string(&stored)?),
    )?;
    Ok(TestsStep::Result(Some(result)))
}
