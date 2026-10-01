//! Run-time guards of the step loop: the policy is applied again when a task
//! runs, a step asks for no more than the task may still spend, and a running
//! test is stoppable.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crate::neppy::security::policy::AutonomyLevel;

use super::super::model::{ModelFailure, ModelReply, StepModel};
use super::super::planning::yields_for_test;
use super::super::test_support::*;
use super::*;
use crate::neppy::inference::local::service::mlx_admin::gate::{is_pinned, GateError};

const SEARCH: &str = "pub fn alpha() {}";
const REPLACE: &str = "pub fn alpha() { /* v2 */ }";

async fn run(env: &RunEnv, id: &str) -> Result<RunOutcome> {
    run_task(env, id, &StopSignal::new()).await
}

fn never_done() -> String {
    plan_json("progress", &[], false, "more", false)
}

// ---- the task's token budget is a ceiling --------------------------------

#[tokio::test]
async fn each_step_asks_for_no_more_than_the_task_has_left() {
    let mut fx = Fixture::new();
    fx.cfg.step_max_tokens = 200;
    fx.cfg.task_max_completion_tokens = 250;
    let id = fx.new_task(false, false);
    // `ok_reply` reports 100 completion tokens each.
    let model = ScriptedModel::new((0..6).map(|_| ok_reply(&never_done())).collect());
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::BudgetExhausted)
    );
    // 250 left -> 200 (the step limit), then 150, then 50: the task cannot
    // overshoot by a whole step's worth.
    assert_eq!(*model.max_tokens.lock(), vec![200, 150, 50]);
}

#[test]
fn the_step_cap_is_the_smaller_of_the_step_limit_and_what_is_left() {
    use super::super::planning::step_token_cap;
    assert_eq!(step_token_cap(1536, 10_000), 1536);
    assert_eq!(step_token_cap(1536, 1536), 1536);
    assert_eq!(step_token_cap(1536, 700), 700);
    assert_eq!(step_token_cap(1536, 1), 1);
    assert_eq!(
        step_token_cap(1536, u64::MAX),
        1536,
        "no truncation on a huge budget"
    );
}

#[tokio::test]
async fn a_resumed_step_with_nothing_left_to_spend_makes_no_call() {
    let mut fx = Fixture::new();
    fx.cfg.task_max_completion_tokens = 500;
    let id = fx.new_task(false, false);
    // A step that started, before the budget ran out.
    fx.store.begin_step(&id, 1).unwrap();
    fx.store.add_tokens(&id, 500).unwrap();
    let model = ScriptedModel::new(vec![ok_reply(&never_done())]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::BudgetExhausted)
    );
    assert_eq!(model.calls(), 0, "it used to call and overshoot");
}

// ---- the policy is applied again at run time -----------------------------

#[tokio::test]
async fn a_root_that_is_no_longer_allowed_fails_the_task_before_any_model_call() {
    let fx = Fixture::new();
    let id = fx.new_task(true, false);
    let model = ScriptedModel::new(vec![ok_reply(&plan_json(
        "s",
        &[("src/lib.rs", SEARCH, REPLACE)],
        false,
        "n",
        true,
    ))]);
    let mut env = fx.plain_env(model.clone());
    // The write grant was withdrawn after the task was accepted.
    let mut policy = fx.policy();
    policy.trusted_roots.clear();
    env.policy = Arc::new(policy);
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Failed)
    );
    assert_eq!(model.calls(), 0);
    assert_eq!(fx.lib(), "pub fn alpha() {}\n");
    let task = fx.store.require_task(&id).unwrap();
    assert!(
        task.error.unwrap_or_default().contains("no longer allowed"),
        "the reason is on the task"
    );
}

#[tokio::test]
async fn a_root_that_lost_its_repository_is_not_edited_but_can_still_be_read() {
    let fx = Fixture::new();
    std::fs::remove_dir_all(fx.project.path().join(".git")).unwrap();
    let edit_task = fx.new_task(true, false);
    let read_task = fx.new_task(false, false);
    let model = ScriptedModel::new(vec![ok_reply(&plan_json("s", &[], false, "n", true))]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &edit_task).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Failed)
    );
    assert_eq!(
        run(&env, &read_task).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
}

#[tokio::test]
async fn a_command_the_gate_would_ask_about_is_recorded_not_run_under_supervised() {
    let mut fx = Fixture::new();
    fx.autonomy = AutonomyLevel::Supervised;
    let id = fx.new_task(false, true);
    let model = ScriptedModel::new(vec![ok_reply(&plan_json("s", &[], true, "n", true))]);
    let env = fx.plain_env(model);
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(
        fx.test_runs(),
        0,
        "nothing was run, and nothing was assumed approved"
    );
    let last = fx.store.require_task(&id).unwrap().last_test.unwrap();
    assert_eq!(last.exit_code, TEST_REFUSED);
}

#[tokio::test]
async fn the_test_command_is_charged_to_the_action_budget() {
    let fx = Fixture::new();
    let id = fx.new_task(false, true);
    let model = ScriptedModel::new(vec![
        ok_reply(&plan_json("s1", &[], true, "n", false)),
        ok_reply(&plan_json("s2", &[], true, "n", true)),
    ]);
    let mut env = fx.plain_env(model);
    let mut policy = fx.policy();
    policy.max_actions_per_hour = 1;
    env.policy = Arc::new(policy);
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(fx.test_runs(), 1, "the second request hit the limit");
    let last = fx.store.require_task(&id).unwrap().last_test.unwrap();
    assert_eq!(last.exit_code, TEST_REFUSED);
}

// ---- a running test is stoppable -----------------------------------------

fn slow_test_task(fx: &Fixture) -> String {
    let root = fx.project.path().canonicalize().unwrap();
    let spec = TaskSpec {
        project_root: root.clone(),
        goal: "g".into(),
        allow_edits: false,
        test_command: Some(format!(
            "echo run >> {}; sleep 300",
            fx.counter_path().display()
        )),
        max_steps: None,
    };
    fx.store
        .create_task(&spec, &root.to_string_lossy(), fx.cfg.max_steps)
        .unwrap()
        .id
}

async fn until_test_starts(fx: &Fixture) {
    for _ in 0..200 {
        if fx.test_runs() > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the test command never started");
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_a_task_kills_its_running_test() {
    let fx = Fixture::new();
    let id = slow_test_task(&fx);
    let model = ScriptedModel::new(vec![ok_reply(&plan_json("s", &[], true, "n", true))]);
    let env = fx.plain_env(model);
    let stop = StopSignal::new();
    let cancel = stop.cancel.clone();
    let task_id = id.clone();
    let handle = tokio::spawn(async move { run_task(&env, &task_id, &stop).await });
    until_test_starts(&fx).await;
    let started = Instant::now();
    cancel.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("the run ended promptly")
        .unwrap()
        .unwrap();
    assert_eq!(outcome, RunOutcome::Finished(TaskStatus::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        fx.store.require_task(&id).unwrap().status,
        TaskStatus::Cancelled
    );
}

#[cfg(unix)]
#[tokio::test]
async fn disabling_the_assistant_stops_a_running_test_and_pauses_the_task() {
    let fx = Fixture::new();
    let id = slow_test_task(&fx);
    let model = ScriptedModel::new(vec![ok_reply(&plan_json("s", &[], true, "n", true))]);
    let env = fx.plain_env(model);
    let stop = StopSignal::new();
    let enabled = Arc::clone(&stop.enabled);
    let task_id = id.clone();
    let handle = tokio::spawn(async move { run_task(&env, &task_id, &stop).await });
    until_test_starts(&fx).await;
    enabled.store(false, Ordering::SeqCst);
    let outcome = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("the run ended promptly")
        .unwrap()
        .unwrap();
    assert_eq!(outcome, RunOutcome::Paused);
    assert_eq!(
        fx.store.require_task(&id).unwrap().status,
        TaskStatus::Paused
    );
    // The test never finished, so its ledger row is still open and a resume
    // runs it again rather than treating it as done.
    let step = fx.store.latest_step(&id).unwrap().unwrap();
    assert!(step.state < StepState::Tested);
    let rows = fx.store.effects_of_step(&id, 1).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, EffectStatus::Intent);
}

// ---- yielding is bounded, and cancelled calls are charged ----------------

/// Fails the first `fail_first` calls with a gate error, then answers.
struct GateModel {
    fail_first: usize,
    failure: fn() -> ModelFailure,
    calls: std::sync::atomic::AtomicUsize,
    pinned: parking_lot::Mutex<Vec<bool>>,
    max_tokens: parking_lot::Mutex<Vec<u32>>,
    reply: String,
}

impl GateModel {
    fn new(fail_first: usize, failure: fn() -> ModelFailure) -> Arc<Self> {
        Arc::new(Self {
            fail_first,
            failure,
            calls: std::sync::atomic::AtomicUsize::new(0),
            pinned: parking_lot::Mutex::new(Vec::new()),
            max_tokens: parking_lot::Mutex::new(Vec::new()),
            reply: plan_json("s", &[], false, "n", true),
        })
    }
}

#[async_trait::async_trait]
impl StepModel for GateModel {
    async fn complete(
        &self,
        _system: &str,
        _user: &str,
        max_tokens: u32,
    ) -> std::result::Result<ModelReply, ModelFailure> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.pinned.lock().push(is_pinned());
        self.max_tokens.lock().push(max_tokens);
        if n < self.fail_first {
            return Err((self.failure)());
        }
        Ok(ModelReply {
            text: self.reply.clone(),
            prompt_tokens: Some(100),
            completion_tokens: Some(10),
        })
    }
}

fn yielded() -> ModelFailure {
    ModelFailure::Gate(GateError::Yielded)
}

#[tokio::test]
async fn after_three_yields_a_step_is_pinned_and_cannot_be_starved() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let model = GateModel::new(3, yielded);
    let env = fx.plain_env(model.clone());
    let mut outcomes = Vec::new();
    for _ in 0..6 {
        let outcome = run(&env, &id).await.unwrap();
        outcomes.push(outcome.clone());
        if matches!(outcome, RunOutcome::Finished(_)) {
            break;
        }
        assert_eq!(yields_for_test(&id, 1), outcomes.len() as u32);
    }
    assert_eq!(
        outcomes.len(),
        4,
        "three yields, then the step goes through"
    );
    assert!(outcomes[..3]
        .iter()
        .all(|o| matches!(o, RunOutcome::Preempted(why) if why == "Yielded")));
    assert_eq!(outcomes[3], RunOutcome::Finished(TaskStatus::Done));
    assert_eq!(
        *model.pinned.lock(),
        vec![false, false, false, true],
        "the fourth call may no longer be cancelled to make room"
    );
    assert_eq!(
        yields_for_test(&id, 1),
        0,
        "forgotten once the plan is stored"
    );
}

#[tokio::test]
async fn yields_are_counted_per_step_not_per_task() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    // Step 1 yields once; step 2 has no yields of its own.
    let first = GateModel::new(1, yielded);
    let env = fx.plain_env(first.clone());
    assert!(matches!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Preempted(_)
    ));
    assert_eq!(yields_for_test(&id, 1), 1);
    assert_eq!(yields_for_test(&id, 2), 0);
}

#[tokio::test]
async fn a_call_cancelled_while_generating_is_charged_at_its_full_max_tokens() {
    let mut fx = Fixture::new();
    fx.cfg.step_max_tokens = 300;
    fx.cfg.task_max_completion_tokens = 500;
    let id = fx.new_task(false, false);
    let model = GateModel::new(10, || ModelFailure::GateAfterStart(GateError::Preempted));
    let env = fx.plain_env(model.clone());

    assert!(matches!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Preempted(_)
    ));
    assert_eq!(
        fx.store.require_task(&id).unwrap().completion_tokens_used,
        300
    );
    // 200 left: the next call asks for 200 and is charged 200.
    assert!(matches!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Preempted(_)
    ));
    assert_eq!(
        fx.store.require_task(&id).unwrap().completion_tokens_used,
        500
    );
    assert_eq!(*model.max_tokens.lock(), vec![300, 200]);
    // Nothing left: the task ends without asking again. The budget held.
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::BudgetExhausted)
    );
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_call_cancelled_before_it_generated_costs_nothing() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let model = GateModel::new(1, || ModelFailure::Gate(GateError::Preempted));
    let env = fx.plain_env(model);
    assert!(matches!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Preempted(_)
    ));
    assert_eq!(
        fx.store.require_task(&id).unwrap().completion_tokens_used,
        0
    );
}
