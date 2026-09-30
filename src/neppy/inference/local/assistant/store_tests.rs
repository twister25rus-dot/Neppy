use super::super::test_support::{spec, store};
use super::*;

fn plan(tag: &str) -> StepPlan {
    StepPlan {
        summary: format!("summary {tag}"),
        decisions: vec![format!("decide {tag}")],
        edits: vec![EditOp {
            path: "a.rs".into(),
            search: "x".repeat(3000),
            replace: "y".repeat(3000),
        }],
        run_tests: true,
        next_step: format!("next {tag}"),
        done: false,
        search_queries: vec!["q".into()],
    }
}

#[test]
fn a_new_task_is_queued_and_round_trips() {
    let (_d, store) = store();
    let task = store.create_task(&spec("do it"), "/p", 8).unwrap();
    assert_eq!(task.status, TaskStatus::Queued);
    assert!(task.allow_edits);
    assert_eq!(task.test_command.as_deref(), Some("true"));
    let loaded = store.require_task(&task.id).unwrap();
    assert_eq!(loaded.goal, "do it");
    assert!(matches!(
        store.require_task("nope"),
        Err(AssistantError::NotFound(_))
    ));
    assert_eq!(store.list_tasks(10).unwrap().len(), 1);
}

#[test]
fn a_plan_is_saved_once_and_counts_its_tokens_once() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    store.begin_step(&task.id, 1).unwrap();
    store.save_plan(&task.id, 1, &plan("a"), 100, 40).unwrap();
    // A replay of the same step must not double-count.
    store.save_plan(&task.id, 1, &plan("b"), 100, 40).unwrap();
    let t = store.require_task(&task.id).unwrap();
    assert_eq!(t.completion_tokens_used, 40);
    let step = store.load_step(&task.id, 1).unwrap().unwrap();
    assert_eq!(step.state, StepState::Planned);
    assert_eq!(step.plan.unwrap().summary, "summary a", "first plan wins");
}

#[test]
fn an_oversized_plan_is_refused() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    store.begin_step(&task.id, 1).unwrap();
    let mut big = plan("x");
    big.edits[0].search = "z".repeat(PLAN_JSON_MAX);
    assert!(matches!(
        store.save_plan(&task.id, 1, &big, 1, 1),
        Err(AssistantError::Invalid(_))
    ));
}

#[test]
fn step_state_only_moves_forward() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    store.begin_step(&task.id, 1).unwrap();
    store
        .set_step_state(&task.id, 1, StepState::Applying)
        .unwrap();
    store
        .set_step_state(&task.id, 1, StepState::Planned)
        .unwrap();
    assert_eq!(
        store.load_step(&task.id, 1).unwrap().unwrap().state,
        StepState::Applying
    );
}

#[test]
fn effect_keys_are_unique_and_intent_is_not_overwritten() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    let rec = EffectRecord {
        effect_key: "k1".into(),
        task_id: task.id.clone(),
        step_no: 1,
        kind: EffectKind::Edit,
        path: "a.rs".into(),
        pre_sha: "pre".into(),
        post_sha: "post".into(),
        status: EffectStatus::Intent,
        result: None,
    };
    store.record_effect_intent(&rec).unwrap();
    store
        .mark_effect("k1", EffectStatus::Applied, None)
        .unwrap();
    store
        .record_effect_intent(&EffectRecord {
            post_sha: "other".into(),
            ..rec.clone()
        })
        .unwrap();
    let got = store.get_effect("k1").unwrap().unwrap();
    assert_eq!(got.status, EffectStatus::Applied);
    assert_eq!(got.post_sha, "post");
    assert_eq!(store.count("effects"), 1);
}

#[test]
fn completing_a_step_updates_the_task_in_one_transaction() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    store.begin_step(&task.id, 1).unwrap();
    store.save_plan(&task.id, 1, &plan("a"), 10, 5).unwrap();
    store
        .record_effect_intent(&EffectRecord {
            effect_key: "e".into(),
            task_id: task.id.clone(),
            step_no: 1,
            kind: EffectKind::Edit,
            path: "a.rs".into(),
            pre_sha: "a".into(),
            post_sha: "b".into(),
            status: EffectStatus::Applied,
            result: None,
        })
        .unwrap();
    let update = TaskUpdate {
        summary: "s1".into(),
        new_decisions: vec!["d1".into(), "d1".into()],
        new_changed_files: vec!["a.rs".into()],
        last_test: Some(TestResult {
            exit_code: 0,
            duration_ms: 3,
            tail: "ok".into(),
            timed_out: false,
        }),
        next_step: "n1".into(),
        finish: None,
    };
    store.complete_step(&task.id, 1, &update).unwrap();
    // Completing twice must not count the step twice.
    store.complete_step(&task.id, 1, &update).unwrap();

    let t = store.require_task(&task.id).unwrap();
    assert_eq!(t.steps_done, 1);
    assert_eq!(t.summary, "s1");
    assert_eq!(t.decisions, vec!["d1".to_string()]);
    assert_eq!(t.changed_files, vec!["a.rs".to_string()]);
    assert_eq!(t.next_step, "n1");
    assert!(t.last_test.unwrap().passed());
    assert_eq!(
        store.load_step(&task.id, 1).unwrap().unwrap().state,
        StepState::Completed
    );
    assert_eq!(
        store.get_effect("e").unwrap().unwrap().status,
        EffectStatus::Done
    );
}

#[test]
fn stored_values_are_bounded() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    store.begin_step(&task.id, 1).unwrap();
    let many = |n: usize| (0..n).map(|i| format!("decision {i}")).collect::<Vec<_>>();
    store
        .complete_step(
            &task.id,
            1,
            &TaskUpdate {
                summary: "Sentence one. ".repeat(500),
                new_decisions: many(60),
                new_changed_files: (0..400).map(|i| format!("f{i}.rs")).collect(),
                last_test: Some(TestResult {
                    exit_code: 1,
                    duration_ms: 1,
                    tail: "t".repeat(10_000),
                    timed_out: false,
                }),
                next_step: "n".repeat(5000),
                finish: None,
            },
        )
        .unwrap();
    let t = store.require_task(&task.id).unwrap();
    assert!(t.summary.len() <= SUMMARY_MAX);
    assert_eq!(t.decisions.len(), DECISIONS_MAX);
    assert_eq!(t.decisions.last().unwrap(), "decision 59", "newest kept");
    assert_eq!(t.changed_files.len(), CHANGED_FILES_MAX);
    assert_eq!(t.changed_files.last().unwrap(), "f399.rs");
    assert!(t.last_test.unwrap().tail.len() <= TEST_TAIL_STORE);
    assert!(t.next_step.len() <= NEXT_STEP_MAX);
}

#[test]
fn cap_summary_drops_the_oldest_sentences_first() {
    let text = format!("OLDEST. {} NEWEST.", "filler sentence. ".repeat(200));
    let capped = cap_summary(&text);
    assert!(capped.len() <= SUMMARY_MAX);
    assert!(capped.ends_with("NEWEST."));
    assert!(!capped.contains("OLDEST"));
    assert_eq!(cap_summary("  short  "), "short");
    assert!(cap_summary(&"x".repeat(5000)).len() <= SUMMARY_MAX);
}

#[test]
fn finishing_a_task_compacts_its_stored_plans() {
    let (_d, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap();
    store.begin_step(&task.id, 1).unwrap();
    store.save_plan(&task.id, 1, &plan("a"), 10, 5).unwrap();
    let before = store.load_step(&task.id, 1).unwrap().unwrap().plan.unwrap();
    assert!(before.edits[0].search.len() > 2000);
    store
        .complete_step(
            &task.id,
            1,
            &TaskUpdate {
                summary: "s".into(),
                finish: Some(TaskStatus::Done),
                ..TaskUpdate::default()
            },
        )
        .unwrap();
    let after = store.load_step(&task.id, 1).unwrap().unwrap().plan.unwrap();
    assert!(after.edits[0].search.is_empty());
    assert_eq!(after.edits[0].path, "a.rs");
    assert_eq!(
        store.require_task(&task.id).unwrap().status,
        TaskStatus::Done
    );
}

#[test]
fn a_restart_interrupts_running_and_queued_tasks_only() {
    let (_d, store) = store();
    let running = store.create_task(&spec("r"), "/p", 8).unwrap();
    let queued = store.create_task(&spec("q"), "/p", 8).unwrap();
    let done = store.create_task(&spec("d"), "/p", 8).unwrap();
    store
        .set_status(&running.id, TaskStatus::Running, None)
        .unwrap();
    store.set_status(&done.id, TaskStatus::Done, None).unwrap();
    let mut ids = store.mark_interrupted_on_boot().unwrap();
    ids.sort();
    let mut want = vec![running.id.clone(), queued.id.clone()];
    want.sort();
    assert_eq!(ids, want);
    assert_eq!(
        store.require_task(&running.id).unwrap().status,
        TaskStatus::Interrupted
    );
    assert_eq!(
        store.require_task(&done.id).unwrap().status,
        TaskStatus::Done
    );
}

#[test]
fn transition_only_moves_from_the_listed_states() {
    let (_d, store) = store();
    let t = store.create_task(&spec("g"), "/p", 8).unwrap();
    assert!(!store
        .transition(&t.id, &[TaskStatus::Running], TaskStatus::Cancelled)
        .unwrap());
    assert!(store
        .transition(&t.id, &[TaskStatus::Queued], TaskStatus::Cancelled)
        .unwrap());
}

#[test]
fn retention_prunes_old_and_excess_finished_tasks_but_never_live_ones() {
    let (_d, store) = store();
    let mut finished = Vec::new();
    for i in 0..6 {
        let t = store.create_task(&spec(&format!("t{i}")), "/p", 8).unwrap();
        store.set_status(&t.id, TaskStatus::Done, None).unwrap();
        store.begin_step(&t.id, 1).unwrap();
        finished.push(t.id);
    }
    let live = store.create_task(&spec("live"), "/p", 8).unwrap();
    store
        .set_status(&live.id, TaskStatus::Running, None)
        .unwrap();
    let now = now_ms();
    // Oldest first.
    for (i, id) in finished.iter().enumerate() {
        store.backdate(id, now - 1000 * (10 - i as i64));
    }
    // Excess: keep the newest 3.
    let removed = store.prune(3, 30, now).unwrap();
    assert_eq!(removed, 3);
    assert!(store.load_task(&finished[0]).unwrap().is_none());
    assert!(store.load_task(&finished[5]).unwrap().is_some());
    assert_eq!(store.count("steps"), 3, "steps go with their task");

    // Age: anything older than 30 days goes, even within the count.
    store.backdate(&finished[5], now - 31 * 86_400_000);
    assert_eq!(store.prune(50, 30, now).unwrap(), 1);
    assert!(store.load_task(&live.id).unwrap().is_some());
}
