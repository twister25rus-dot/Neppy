//! Controller and API fixes from review: write-root policy at accept time, one
//! queue entry per task, and parked tasks coming back after a restart.

use std::path::{Path, PathBuf};

use crate::neppy::security::{TrustedAccess, TrustedRoot};

use super::*;

/// The project gets a repository marker and a read-write grant, which is what a
/// root that is to be edited needs.
fn trust_project(e: &mut Env) {
    std::fs::create_dir_all(e.project.path().join(".git")).unwrap();
    let mut policy = (*e.ctx.policy).clone();
    policy.trusted_roots = vec![TrustedRoot {
        path: e
            .project
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        access: TrustedAccess::ReadWrite,
    }];
    e.ctx.policy = Arc::new(policy);
}

fn edit_spec(e: &Env) -> TaskSpec {
    TaskSpec {
        allow_edits: true,
        ..e.spec("edit it")
    }
}

#[tokio::test]
async fn a_task_may_edit_only_a_trusted_git_project() {
    let mut e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    // Not trusted, not a repository.
    let err = api::start_task(&e.ctx, edit_spec(&e))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("forbidden") || err.contains("trusted_roots"),
        "{err}"
    );
    assert_eq!(
        e.ctx.store.list_tasks(10).unwrap().len(),
        0,
        "no row, no slot"
    );

    // Trusted, still not a repository.
    trust_project(&mut e);
    std::fs::remove_dir_all(e.project.path().join(".git")).unwrap();
    let err = api::start_task(&e.ctx, edit_spec(&e))
        .unwrap_err()
        .to_string();
    assert!(err.contains("git repository"), "{err}");

    // Both.
    std::fs::create_dir_all(e.project.path().join(".git")).unwrap();
    let task = api::start_task(&e.ctx, edit_spec(&e)).unwrap();
    assert!(task.allow_edits);
    // Reading never needed either.
    let bare = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    assert!(api::start_task(&bare.ctx, bare.spec("just read")).is_ok());
}

#[tokio::test]
async fn system_persistence_and_home_roots_are_refused_for_edits() {
    let mut e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    trust_project(&mut e);
    let mut home_paths = vec![PathBuf::from("/"), PathBuf::from("/usr/local")];
    if let Some(home) = dirs::home_dir() {
        home_paths.push(home.join("Library/LaunchAgents"));
        home_paths.push(home);
    }
    for root in home_paths {
        let spec = TaskSpec {
            project_root: root.clone(),
            ..edit_spec(&e)
        };
        assert!(
            api::start_task(&e.ctx, spec).is_err(),
            "{} must not be accepted for edits",
            root.display()
        );
    }
    assert_eq!(e.ctx.store.list_tasks(10).unwrap().len(), 0);
}

#[tokio::test]
async fn the_home_directory_is_not_a_project_even_to_read() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    for root in ["/".to_string()].into_iter().chain(
        dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .into_iter(),
    ) {
        let spec = TaskSpec {
            project_root: root.clone().into(),
            ..e.spec("read")
        };
        assert!(api::start_task(&e.ctx, spec).is_err(), "{root}");
        assert!(
            api::index_status(&e.ctx, Path::new(&root)).is_err(),
            "{root}"
        );
    }
}

#[tokio::test]
async fn supervised_refuses_a_test_command_that_needs_approval_when_the_task_is_accepted() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Supervised);
    let err = api::start_task(
        &e.ctx,
        TaskSpec {
            test_command: Some("touch built.marker".into()),
            ..e.spec("g")
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("test_command refused"), "{err}");
    assert!(err.contains("approval"), "{err}");
    // A read-only command is fine in the same tier.
    assert!(api::start_task(
        &e.ctx,
        TaskSpec {
            test_command: Some("git status".into()),
            ..e.spec("g")
        },
    )
    .is_ok());
}

#[tokio::test]
async fn a_forbidden_path_argument_in_the_test_command_is_refused_up_front() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    let err = api::start_task(
        &e.ctx,
        TaskSpec {
            test_command: Some("cat ~/.ssh/id_ed25519".into()),
            ..e.spec("g")
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("forbidden path argument"), "{err}");
}

// ---- one queue entry per task --------------------------------------------

#[tokio::test]
async fn a_task_the_controller_already_holds_is_not_queued_again() {
    let e = env(4, FakeRunner::new(true, vec![]), AutonomyLevel::Full);
    let first = api::start_task(&e.ctx, e.spec("one")).unwrap();
    let second = api::start_task(&e.ctx, e.spec("two")).unwrap();
    until("the first to run", || e.runner.started().len() == 1).await;
    assert_eq!(e.ctx.controller.pending(), 2);

    // Resubmitting either id (a resume racing the boot pass) is absorbed.
    for id in [&first.id, &second.id] {
        let slot = e.ctx.controller.reserve().unwrap();
        slot.submit(QueueItem {
            workspace: e.ctx.workspace.clone(),
            task_id: id.clone(),
        })
        .unwrap();
    }
    assert_eq!(
        e.ctx.controller.pending(),
        2,
        "no place was kept for a duplicate"
    );
    let mut tracked = e.ctx.controller.tracked_ids();
    tracked.sort();
    let mut expect = vec![first.id.clone(), second.id.clone()];
    expect.sort();
    assert_eq!(tracked, expect);

    e.runner.release.add_permits(10);
    until("idle", || e.ctx.controller.pending() == 0).await;
    assert_eq!(e.runner.started().len(), 2, "each task ran once");
    assert!(e.ctx.controller.tracked_ids().is_empty());
}

#[tokio::test]
async fn the_boot_pass_leaves_alone_tasks_accepted_since_the_controller_started() {
    let e = env(4, FakeRunner::new(true, vec![]), AutonomyLevel::Full);
    let a = api::start_task(&e.ctx, e.spec("accepted before the boot pass")).unwrap();
    let b = api::start_task(&e.ctx, e.spec("queued behind it")).unwrap();
    until("the first to run", || e.runner.started().len() == 1).await;

    // Both are `queued`/`running` in the database, as a stale task from a
    // previous process would be, but this process holds them.
    assert_eq!(api::resume_after_restart(&e.ctx).unwrap(), 0);
    for id in [&a.id, &b.id] {
        let status = e.ctx.store.require_task(id).unwrap().status;
        assert_ne!(
            status,
            TaskStatus::Interrupted,
            "{id} was marked interrupted"
        );
    }
    e.runner.release.add_permits(10);
    until("idle", || e.ctx.controller.pending() == 0).await;
    assert_eq!(e.runner.started().len(), 2, "neither ran twice");
}

// ---- parked tasks come back at boot --------------------------------------

#[tokio::test]
async fn a_task_a_pressure_give_up_left_interrupted_is_requeued_at_boot() {
    let runner = FakeRunner::new(false, vec![]);
    let e = env(6, runner.clone(), AutonomyLevel::Full);
    let parked = e.ctx.store.create_task(&e.spec("parked"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&parked.id, TaskStatus::Interrupted, Some("gave up waiting"))
        .unwrap();
    let paused = e.ctx.store.create_task(&e.spec("paused"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&paused.id, TaskStatus::Paused, None)
        .unwrap();

    assert_eq!(api::resume_after_restart(&e.ctx).unwrap(), 1);
    until("the parked task to run", || {
        runner.started().contains(&parked.id)
    })
    .await;
    assert!(
        !runner.started().contains(&paused.id),
        "a paused task waits for an explicit enable"
    );
}

#[tokio::test]
async fn parked_tasks_stay_parked_while_the_assistant_is_disabled() {
    let runner = FakeRunner::new(false, vec![]);
    let e = env(6, runner.clone(), AutonomyLevel::Full);
    e.ctx.controller.set_enabled(false);
    let parked = e.ctx.store.create_task(&e.spec("parked"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&parked.id, TaskStatus::Interrupted, None)
        .unwrap();
    assert_eq!(api::resume_after_restart(&e.ctx).unwrap(), 0);
    assert!(runner.started().is_empty());
    assert_eq!(
        e.ctx.store.require_task(&parked.id).unwrap().status,
        TaskStatus::Interrupted
    );
}

// ---- yielding is not failing ---------------------------------------------

#[tokio::test]
async fn yielding_to_chat_does_not_use_up_the_requeue_budget() {
    let yields = MAX_REQUEUES as usize + 5;
    let mut outcomes: Vec<RunOutcome> = (0..yields)
        .map(|_| RunOutcome::Preempted("Yielded".into()))
        .collect();
    outcomes.push(RunOutcome::Finished(TaskStatus::Done));
    let runner = FakeRunner::new(false, outcomes);
    let e = env(4, runner.clone(), AutonomyLevel::Full);
    api::start_task(&e.ctx, e.spec("g")).unwrap();
    until("every run", || runner.started().len() == yields + 1).await;
    until("idle", || e.ctx.controller.pending() == 0).await;
    assert_eq!(
        runner.gave_up.load(Ordering::SeqCst),
        0,
        "a task that keeps making way for chat is healthy"
    );
}

#[test]
fn only_a_yield_is_recognised_as_a_yield() {
    assert!(is_yield("Yielded"));
    for other in ["Preempted", "Paused(Critical)", "Busy", ""] {
        assert!(!is_yield(other), "{other}");
    }
}
