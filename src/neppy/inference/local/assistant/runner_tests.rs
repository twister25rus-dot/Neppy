use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::neppy::security::policy::AutonomyLevel;

use super::super::model::ModelFailure;
use super::super::test_support::*;
use super::*;
use crate::neppy::inference::local::service::mlx_admin::gate::GateError;

const SEARCH: &str = "pub fn alpha() {}";
// Contains its own search text: applying it twice would be visible.
const REPLACE: &str = "pub fn alpha() { /* v2 */ }";

fn step_one() -> String {
    plan_json(
        "step one summary",
        &[("src/lib.rs", SEARCH, REPLACE)],
        true,
        "verify it",
        false,
    )
}

fn step_two() -> String {
    plan_json("step two summary", &[], false, "nothing left", true)
}

async fn run(env: &RunEnv, id: &str) -> Result<RunOutcome> {
    run_task(env, id, &StopSignal::new()).await
}

#[tokio::test]
async fn a_two_step_task_edits_tests_and_finishes() {
    let fx = Fixture::new();
    let id = fx.new_task(true, true);
    let model = ScriptedModel::new(vec![ok_reply(&step_one()), ok_reply(&step_two())]);
    let env = fx.plain_env(model.clone());

    let outcome = run(&env, &id).await.unwrap();
    assert_eq!(outcome, RunOutcome::Finished(TaskStatus::Done));
    assert_eq!(fx.lib(), "pub fn alpha() { /* v2 */ }\n");
    assert_eq!(model.calls(), 2);
    assert_eq!(fx.test_runs(), 1);
    let task = fx.store.require_task(&id).unwrap();
    assert_eq!(task.status, TaskStatus::Done);
    assert_eq!(task.steps_done, 2);
    assert_eq!(task.summary, "step two summary");
    assert_eq!(task.changed_files, vec!["src/lib.rs".to_string()]);
    assert!(task.last_test.unwrap().passed());
    assert_eq!(task.completion_tokens_used, 200);
}

#[tokio::test]
async fn every_crash_point_resumes_without_repeating_an_edit_test_or_model_call() {
    let points = [
        FaultPoint::Planned,
        FaultPoint::Intent(0),
        FaultPoint::Write(0),
        FaultPoint::Applied,
        FaultPoint::TestIntent,
        FaultPoint::Tested,
    ];
    for point in points {
        let fx = Fixture::new();
        let id = fx.new_task(true, true);
        // Exactly the planned steps' worth of replies: a re-ask would hit
        // "script exhausted" and fail the run.
        let model = ScriptedModel::new(vec![ok_reply(&step_one()), ok_reply(&step_two())]);
        let crash = CrashOnce::new(point);
        let env = fx.env(model.clone(), crash.clone());

        let first = run(&env, &id).await;
        assert!(
            matches!(first, Err(AssistantError::Interrupted(_))),
            "{point:?}: {first:?}"
        );
        assert!(crash.fired(), "{point:?} never reached");

        // A restart: nothing is running, whatever the database says.
        fx.store.mark_interrupted_on_boot().unwrap();
        let env = fx.env(model.clone(), Arc::new(super::super::faults::NoFaults));
        let second = run(&env, &id).await.unwrap();
        assert_eq!(second, RunOutcome::Finished(TaskStatus::Done), "{point:?}");

        assert_eq!(
            fx.lib(),
            "pub fn alpha() { /* v2 */ }\n",
            "{point:?}: edit applied once"
        );
        assert_eq!(model.calls(), 2, "{point:?}: no replan");
        assert_eq!(fx.test_runs(), 1, "{point:?}: test ran once");
        let task = fx.store.require_task(&id).unwrap();
        assert_eq!(task.steps_done, 2, "{point:?}");
        assert_eq!(
            task.completion_tokens_used, 200,
            "{point:?}: tokens counted once"
        );
    }
}

#[tokio::test]
async fn a_preempted_call_keeps_the_step_and_the_next_run_asks_again() {
    let fx = Fixture::new();
    let id = fx.new_task(true, false);
    let model = ScriptedModel::new(vec![
        Err(ModelFailure::Gate(GateError::Preempted)),
        ok_reply(&plan_json(
            "s",
            &[("src/lib.rs", SEARCH, REPLACE)],
            false,
            "n",
            true,
        )),
    ]);
    let env = fx.plain_env(model.clone());
    let first = run(&env, &id).await.unwrap();
    assert!(matches!(first, RunOutcome::Preempted(_)), "{first:?}");
    let task = fx.store.require_task(&id).unwrap();
    assert_eq!(task.status, TaskStatus::Paused);
    assert_eq!(
        fx.store.load_step(&id, 1).unwrap().unwrap().state,
        StepState::Started,
        "nothing durable happened, so the model is asked again"
    );
    assert_eq!(fx.lib(), "pub fn alpha() {}\n");

    let second = run(&env, &id).await.unwrap();
    assert_eq!(second, RunOutcome::Finished(TaskStatus::Done));
    assert_eq!(model.calls(), 2);
    assert_eq!(fx.lib(), "pub fn alpha() { /* v2 */ }\n");
}

#[tokio::test]
async fn an_edit_that_no_longer_matches_is_a_recorded_conflict_and_the_task_goes_on() {
    let fx = Fixture::new();
    let id = fx.new_task(true, false);
    let model = ScriptedModel::new(vec![
        ok_reply(&plan_json(
            "s1",
            &[("src/lib.rs", "text that is not there", "x")],
            false,
            "retry",
            false,
        )),
        ok_reply(&step_two()),
    ]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(fx.lib(), "pub fn alpha() {}\n", "never overwritten");
    let effects = fx.store.effects_of_step(&id, 1).unwrap();
    assert_eq!(effects[0].status, EffectStatus::Conflict);
    let task = fx.store.require_task(&id).unwrap();
    assert!(
        task.decisions
            .iter()
            .any(|d| d.contains("not applied") && d.contains("conflict")),
        "the model is told: {:?}",
        task.decisions
    );
    // and step 2's prompt carries that note
    assert!(model.prompts.lock()[1].contains("not applied"));
}

#[tokio::test]
async fn edits_are_refused_when_the_task_or_the_tier_does_not_allow_them() {
    for (allow_edits, level) in [
        (false, AutonomyLevel::Full),
        (true, AutonomyLevel::ReadOnly),
    ] {
        let mut fx = Fixture::new();
        fx.autonomy = level;
        let id = fx.new_task(allow_edits, false);
        let model = ScriptedModel::new(vec![ok_reply(&plan_json(
            "s",
            &[("src/lib.rs", SEARCH, REPLACE)],
            false,
            "n",
            true,
        ))]);
        let env = fx.plain_env(model);
        assert_eq!(
            run(&env, &id).await.unwrap(),
            RunOutcome::Finished(TaskStatus::Done)
        );
        assert_eq!(fx.lib(), "pub fn alpha() {}\n");
        assert_eq!(
            fx.store.effects_of_step(&id, 1).unwrap()[0].status,
            EffectStatus::Refused
        );
    }
}

#[tokio::test]
async fn the_completion_token_budget_ends_the_task_with_a_checkpoint() {
    let mut fx = Fixture::new();
    fx.cfg.task_max_completion_tokens = 150;
    let id = fx.new_task(false, false);
    let never_done = plan_json("progress so far", &[], false, "keep going", false);
    let model = ScriptedModel::new(vec![
        ok_reply(&never_done),
        ok_reply(&never_done),
        ok_reply(&never_done),
    ]);
    let env = fx.plain_env(model.clone());
    let outcome = run(&env, &id).await.unwrap();
    assert_eq!(outcome, RunOutcome::Finished(TaskStatus::BudgetExhausted));
    let task = fx.store.require_task(&id).unwrap();
    assert_eq!(
        task.steps_done, 2,
        "100 < 150 allows a second step; 200 >= 150 stops"
    );
    assert_eq!(model.calls(), 2);
    assert_eq!(task.summary, "progress so far", "the checkpoint is kept");
    assert_eq!(task.next_step, "keep going");
}

#[tokio::test]
async fn the_step_limit_ends_the_task() {
    let mut fx = Fixture::new();
    fx.cfg.max_steps = 3;
    let id = fx.new_task(false, false);
    let model = ScriptedModel::new(
        (0..10)
            .map(|_| ok_reply(&plan_json("s", &[], false, "more", false)))
            .collect(),
    );
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::BudgetExhausted)
    );
    assert_eq!(model.calls(), 3);
}

#[tokio::test]
async fn an_unreadable_reply_gets_one_correction_and_both_calls_are_counted() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let model = ScriptedModel::new(vec![
        ok_reply("I think we should look closer."),
        ok_reply(&step_two()),
    ]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(model.calls(), 2);
    assert!(model.prompts.lock()[1].contains("could not be used"));
    let task = fx.store.require_task(&id).unwrap();
    assert_eq!(
        task.completion_tokens_used, 200,
        "the retry is inside the budget"
    );
    assert_eq!(task.steps_done, 1);
}

#[tokio::test]
async fn two_unreadable_replies_fail_the_step_and_a_resume_tries_it_afresh() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let model = ScriptedModel::new(vec![
        ok_reply("nope"),
        ok_reply("still nope"),
        ok_reply(&step_two()),
    ]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Failed)
    );
    let task = fx.store.require_task(&id).unwrap();
    assert!(task.error.unwrap().contains("usable plan"));
    assert_eq!(
        task.completion_tokens_used, 200,
        "the wasted calls still count"
    );
    assert!(task.status.is_resumable());
    // What the resume RPC does before the controller runs it again.
    assert!(fx
        .store
        .transition(&id, &[TaskStatus::Failed], TaskStatus::Queued)
        .unwrap());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
}

#[tokio::test]
async fn transient_model_errors_are_retried_in_place_then_fail() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let model = ScriptedModel::new(vec![
        Err(ModelFailure::Other("connection reset".into())),
        ok_reply(&step_two()),
    ]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(model.calls(), 2);

    let id2 = fx.new_task(false, false);
    let failing = ScriptedModel::new(
        (0..5)
            .map(|_| Err(ModelFailure::Other("down".into())))
            .collect(),
    );
    let env = fx.plain_env(failing.clone());
    assert_eq!(
        run(&env, &id2).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Failed)
    );
    assert_eq!(failing.calls(), 3);
}

#[tokio::test]
async fn an_unadmitted_primary_model_falls_back_once() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let primary = ScriptedModel::new(
        (0..5)
            .map(|_| Err(ModelFailure::Other("[mlx:worker] not enough memory".into())))
            .collect(),
    );
    let fallback = ScriptedModel::new(vec![ok_reply(&step_two())]);
    let mut env = fx.plain_env(primary.clone());
    env.fallback = Some(fallback.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(primary.calls(), 3);
    assert_eq!(fallback.calls(), 1);
}

#[tokio::test]
async fn cancelling_aborts_the_in_flight_call_and_keeps_the_checkpoint() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hang = Arc::new(HangingModel {
        started: tokio::sync::Notify::new(),
        dropped: Arc::clone(&dropped),
    });
    let env = fx.plain_env(hang.clone());
    let stop = StopSignal::new();
    let cancel = stop.cancel.clone();
    let task_id = id.clone();
    let handle = tokio::spawn(async move { run_task(&env, &task_id, &stop).await });
    hang.started.notified().await;
    cancel.cancel();
    let outcome = handle.await.unwrap().unwrap();
    assert_eq!(outcome, RunOutcome::Finished(TaskStatus::Cancelled));
    assert!(
        dropped.load(Ordering::SeqCst),
        "the model call was dropped, releasing its permit"
    );
    assert_eq!(
        fx.store.require_task(&id).unwrap().status,
        TaskStatus::Cancelled
    );
}

#[tokio::test]
async fn disabling_pauses_at_a_step_boundary_and_enabling_continues() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let model = ScriptedModel::new(vec![ok_reply(&step_two())]);
    let env = fx.plain_env(model.clone());
    let stop = StopSignal::new();
    stop.enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        run_task(&env, &id, &stop).await.unwrap(),
        RunOutcome::Paused
    );
    assert_eq!(model.calls(), 0);
    assert_eq!(
        fx.store.require_task(&id).unwrap().status,
        TaskStatus::Paused
    );
    stop.enabled.store(true, Ordering::SeqCst);
    assert_eq!(
        run_task(&env, &id, &stop).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
}

#[tokio::test]
async fn a_prompt_carries_the_notes_and_never_an_earlier_raw_reply() {
    let fx = Fixture::new();
    let id = fx.new_task(false, false);
    let first = serde_json::json!({
        "summary": "kept in the summary",
        "decisions": ["kept as a decision"],
        "edits": [], "run_tests": false, "next_step": "go on", "done": false,
        "search_queries": ["alpha"],
        "scratch": "RAW_REPLY_ONLY_MARKER",
    })
    .to_string();
    let model = ScriptedModel::new(vec![ok_reply(&first), ok_reply(&step_two())]);
    let env = fx.plain_env(model.clone());
    run(&env, &id).await.unwrap();
    let prompts = model.prompts.lock();
    assert!(!prompts[0].contains("kept in the summary"));
    assert!(prompts[1].contains("kept in the summary"));
    assert!(prompts[1].contains("kept as a decision"));
    assert!(!prompts[1].contains("RAW_REPLY_ONLY_MARKER"));
    assert!(
        prompts[1].contains("src/lib.rs"),
        "retrieval supplies fresh excerpts"
    );
}

#[tokio::test]
async fn a_prompt_that_cannot_fit_the_context_fails_before_any_call() {
    let mut fx = Fixture::new();
    fx.cfg.context_limit_tokens = 600;
    fx.cfg.prompt_budget_tokens = 550;
    fx.cfg.step_max_tokens = 500;
    let id = fx.new_task(false, false);
    let model = ScriptedModel::new(vec![ok_reply(&step_two())]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Failed)
    );
    assert_eq!(model.calls(), 0);
    assert!(fx
        .store
        .require_task(&id)
        .unwrap()
        .error
        .unwrap()
        .contains("context limit"));
}

#[tokio::test]
async fn a_test_command_the_policy_blocks_is_recorded_not_run() {
    let mut fx = Fixture::new();
    fx.autonomy = AutonomyLevel::ReadOnly;
    let id = fx.new_task(false, true);
    let model = ScriptedModel::new(vec![ok_reply(&plan_json("s", &[], true, "n", true))]);
    let env = fx.plain_env(model);
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(fx.test_runs(), 0);
    let task = fx.store.require_task(&id).unwrap();
    assert_eq!(task.last_test.unwrap().exit_code, TEST_REFUSED);
}

#[tokio::test]
async fn a_failing_test_keeps_the_task_open_even_if_the_model_says_done() {
    let fx = Fixture::new();
    let root = fx.project.path().canonicalize().unwrap();
    let spec = TaskSpec {
        project_root: root.clone(),
        goal: "g".into(),
        allow_edits: false,
        test_command: Some("exit 3".into()),
        max_steps: None,
    };
    let id = fx
        .store
        .create_task(&spec, &root.to_string_lossy(), 8)
        .unwrap()
        .id;
    let model = ScriptedModel::new(vec![
        ok_reply(&plan_json("s1", &[], true, "fix", true)),
        ok_reply(&plan_json("s2", &[], false, "done now", true)),
    ]);
    let env = fx.plain_env(model.clone());
    assert_eq!(
        run(&env, &id).await.unwrap(),
        RunOutcome::Finished(TaskStatus::Done)
    );
    assert_eq!(model.calls(), 2, "step 1 was not accepted as done");
    assert!(model.prompts.lock()[1].contains("exit 3"));
}
