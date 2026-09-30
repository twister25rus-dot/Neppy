use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::neppy::config::schema::LocalAssistantConfig;
use crate::neppy::security::policy::AutonomyLevel;

use super::super::test_support::store;
use super::*;

/// A runner whose runs block until released, and which records what it saw.
struct FakeRunner {
    started: Mutex<Vec<TaskId>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    release: tokio::sync::Semaphore,
    blocking: bool,
    outcomes: Mutex<VecDeque<RunOutcome>>,
    waited: AtomicUsize,
    gave_up: AtomicUsize,
    cancelled_seen: AtomicUsize,
}

impl FakeRunner {
    fn new(blocking: bool, outcomes: Vec<RunOutcome>) -> Arc<Self> {
        Arc::new(Self {
            started: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            release: tokio::sync::Semaphore::new(0),
            blocking,
            outcomes: Mutex::new(outcomes.into()),
            waited: AtomicUsize::new(0),
            gave_up: AtomicUsize::new(0),
            cancelled_seen: AtomicUsize::new(0),
        })
    }

    fn started(&self) -> Vec<TaskId> {
        self.started.lock().clone()
    }
}

#[async_trait]
impl TaskRunner for FakeRunner {
    async fn run(&self, item: &QueueItem, stop: &StopSignal) -> Result<RunOutcome> {
        self.started.lock().push(item.task_id.clone());
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now, Ordering::SeqCst);
        if self.blocking {
            tokio::select! {
                permit = self.release.acquire() => permit.unwrap().forget(),
                _ = stop.cancel.cancelled() => {
                    self.cancelled_seen.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(self
            .outcomes
            .lock()
            .pop_front()
            .unwrap_or(RunOutcome::Finished(TaskStatus::Done)))
    }

    async fn wait_for_resume(&self, _stop: &StopSignal) -> bool {
        self.waited.fetch_add(1, Ordering::SeqCst);
        true
    }

    async fn give_up(&self, _item: &QueueItem, _why: &str) {
        self.gave_up.fetch_add(1, Ordering::SeqCst);
    }
}

struct Env {
    ctx: ApiCtx,
    runner: Arc<FakeRunner>,
    project: tempfile::TempDir,
    _ws: tempfile::TempDir,
}

fn env(cap: usize, runner: Arc<FakeRunner>, level: AutonomyLevel) -> Env {
    let (ws, st) = store();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(project.path().join("a.rs"), "pub fn alpha_fn() {}\n").unwrap();
    let controller = Controller::spawn(cap, true, runner.clone());
    let ctx = ApiCtx {
        cfg: LocalAssistantConfig::default(),
        workspace: ws.path().to_path_buf(),
        store: Arc::new(st),
        policy: Arc::new(SecurityPolicy {
            autonomy: level,
            workspace_dir: ws.path().to_path_buf(),
            ..SecurityPolicy::default()
        }),
        controller,
    };
    Env {
        ctx,
        runner,
        project,
        _ws: ws,
    }
}

impl Env {
    fn spec(&self, goal: &str) -> TaskSpec {
        TaskSpec {
            project_root: self.project.path().to_path_buf(),
            goal: goal.into(),
            allow_edits: false,
            test_command: None,
            max_steps: None,
        }
    }
}

async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    for _ in 0..500 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn the_queue_is_bounded_and_a_refused_task_leaves_no_row() {
    let e = env(3, FakeRunner::new(true, vec![]), AutonomyLevel::Full);
    for n in 0..3 {
        api::start_task(&e.ctx, e.spec(&format!("task {n}"))).unwrap();
    }
    let err = api::start_task(&e.ctx, e.spec("one too many")).unwrap_err();
    assert!(matches!(err, AssistantError::QueueFull(_)), "{err:?}");
    assert_eq!(e.ctx.store.list_tasks(100).unwrap().len(), 3);
    assert_eq!(e.ctx.controller.pending(), 3);

    // Letting one finish frees a place.
    e.runner.release.add_permits(1);
    until("a place to free", || e.ctx.controller.pending() == 2).await;
    api::start_task(&e.ctx, e.spec("fits now")).unwrap();
    e.runner.release.add_permits(10);
    until("the queue to drain", || e.ctx.controller.pending() == 0).await;
    assert_eq!(
        e.runner.max_active.load(Ordering::SeqCst),
        1,
        "one task at a time"
    );
    assert_eq!(e.runner.started().len(), 4);
}

#[tokio::test]
async fn the_sixteenth_default_slot_is_the_last() {
    let e = env(
        LocalAssistantConfig::default().task_queue_cap,
        FakeRunner::new(true, vec![]),
        AutonomyLevel::Full,
    );
    for n in 0..16 {
        api::start_task(&e.ctx, e.spec(&format!("t{n}"))).unwrap();
    }
    assert!(matches!(
        api::start_task(&e.ctx, e.spec("t16")),
        Err(AssistantError::QueueFull(_))
    ));
    e.runner.release.add_permits(100);
}

#[tokio::test]
async fn a_preempted_task_is_run_again_after_the_wait() {
    let runner = FakeRunner::new(
        false,
        vec![
            RunOutcome::Preempted("Preempted".into()),
            RunOutcome::Preempted("Paused(Critical)".into()),
            RunOutcome::Finished(TaskStatus::Done),
        ],
    );
    let e = env(4, runner.clone(), AutonomyLevel::Full);
    let task = api::start_task(&e.ctx, e.spec("g")).unwrap();
    until("three runs", || runner.started().len() == 3).await;
    until("idle", || e.ctx.controller.pending() == 0).await;
    assert_eq!(runner.waited.load(Ordering::SeqCst), 2);
    assert!(runner.started().iter().all(|id| *id == task.id));
    assert_eq!(runner.gave_up.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_task_preempted_forever_is_given_up_not_spun() {
    let runner = FakeRunner::new(
        false,
        (0..50)
            .map(|_| RunOutcome::Preempted("Preempted".into()))
            .collect(),
    );
    let e = env(4, runner.clone(), AutonomyLevel::Full);
    api::start_task(&e.ctx, e.spec("g")).unwrap();
    until("give up", || runner.gave_up.load(Ordering::SeqCst) == 1).await;
    assert_eq!(runner.started().len() as u32, MAX_REQUEUES + 1);
    until("idle", || e.ctx.controller.pending() == 0).await;
}

#[tokio::test]
async fn start_task_validates_before_queueing() {
    let e = env(4, FakeRunner::new(true, vec![]), AutonomyLevel::Full);
    let bad = |spec: TaskSpec| api::start_task(&e.ctx, spec).unwrap_err().to_string();

    assert!(bad(e.spec("   ")).contains("goal is empty"));
    let mut missing = e.spec("g");
    missing.project_root = "/definitely/not/here".into();
    assert!(bad(missing).contains("project_root"));
    let mut file = e.spec("g");
    file.project_root = e.project.path().join("a.rs");
    assert!(bad(file).contains("not a directory"));
    let mut contains_ws = e.spec("g");
    contains_ws.project_root = e.ctx.workspace.parent().unwrap().to_path_buf();
    assert!(bad(contains_ws).contains("workspace"));
    let mut inside_ws = e.spec("g");
    inside_ws.project_root = e.ctx.workspace.clone();
    assert!(bad(inside_ws).contains("workspace"));
    assert!(bad(TaskSpec {
        goal: "x".repeat(5000),
        ..e.spec("g")
    })
    .contains("too long"));
    assert!(bad(TaskSpec {
        test_command: Some("rm -rf /".into()),
        ..e.spec("g")
    })
    .contains("test_command refused"));
    assert_eq!(e.ctx.store.list_tasks(10).unwrap().len(), 0);
    assert_eq!(
        e.ctx.controller.pending(),
        0,
        "failed validation holds no slot"
    );

    let ok = api::start_task(
        &e.ctx,
        TaskSpec {
            max_steps: Some(1000),
            test_command: Some("  cargo test  ".into()),
            ..e.spec("g")
        },
    )
    .unwrap();
    assert_eq!(
        ok.max_steps, e.ctx.cfg.max_steps,
        "never above the configured limit"
    );
    assert_eq!(ok.test_command.as_deref(), Some("cargo test"));
    let fewer = api::start_task(
        &e.ctx,
        TaskSpec {
            max_steps: Some(2),
            ..e.spec("g")
        },
    )
    .unwrap();
    assert_eq!(fewer.max_steps, 2);
    e.runner.release.add_permits(10);
}

#[tokio::test]
async fn a_read_only_tier_refuses_edits_and_non_read_test_commands_up_front() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::ReadOnly);
    let err = api::start_task(
        &e.ctx,
        TaskSpec {
            allow_edits: true,
            ..e.spec("g")
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("read-only"));
    let err = api::start_task(
        &e.ctx,
        TaskSpec {
            test_command: Some("cargo test".into()),
            ..e.spec("g")
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("test_command refused"));
    assert!(api::start_task(&e.ctx, e.spec("read-only goal")).is_ok());
}

#[tokio::test]
async fn a_disabled_assistant_accepts_no_new_work() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    api::set_enabled(&e.ctx, false).unwrap();
    assert!(matches!(
        api::start_task(&e.ctx, e.spec("g")),
        Err(AssistantError::Disabled)
    ));
    api::set_enabled(&e.ctx, true).unwrap();
    assert!(api::start_task(&e.ctx, e.spec("g")).is_ok());
}

#[tokio::test]
async fn resume_requeues_only_resumable_tasks() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    let task = e.ctx.store.create_task(&e.spec("g"), "/p", 8).unwrap();
    assert!(api::resume_task(&e.ctx, &task.id)
        .unwrap_err()
        .to_string()
        .contains("only paused, interrupted or failed"));
    e.ctx
        .store
        .set_status(&task.id, TaskStatus::Interrupted, None)
        .unwrap();
    let resumed = api::resume_task(&e.ctx, &task.id).unwrap();
    assert_eq!(resumed.status, TaskStatus::Queued);
    until("the run", || e.runner.started() == vec![task.id.clone()]).await;

    e.ctx
        .store
        .set_status(&task.id, TaskStatus::BudgetExhausted, None)
        .unwrap();
    assert!(
        api::resume_task(&e.ctx, &task.id).is_err(),
        "a spent budget is not resumable"
    );
    assert!(matches!(
        api::resume_task(&e.ctx, "nope"),
        Err(AssistantError::NotFound(_))
    ));
}

#[tokio::test]
async fn cancel_marks_the_task_and_aborts_the_running_one() {
    let e = env(4, FakeRunner::new(true, vec![]), AutonomyLevel::Full);
    let running = api::start_task(&e.ctx, e.spec("running")).unwrap();
    let queued = api::start_task(&e.ctx, e.spec("queued")).unwrap();
    until("first to start", || e.runner.started().len() == 1).await;

    let cancelled = api::cancel_task(&e.ctx, &running.id).unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);
    until("abort seen", || {
        e.runner.cancelled_seen.load(Ordering::SeqCst) == 1
    })
    .await;

    let cancelled_queued = api::cancel_task(&e.ctx, &queued.id).unwrap();
    assert_eq!(cancelled_queued.status, TaskStatus::Cancelled);
    // Cancelling a finished task is a no-op, not an error.
    assert_eq!(
        api::cancel_task(&e.ctx, &running.id).unwrap().status,
        TaskStatus::Cancelled
    );
    e.runner.release.add_permits(10);
}

#[tokio::test]
async fn enabling_requeues_paused_tasks_but_not_the_one_the_controller_holds() {
    let runner = FakeRunner::new(true, vec![]);
    let e = env(6, runner.clone(), AutonomyLevel::Full);
    let held = api::start_task(&e.ctx, e.spec("held")).unwrap();
    until("held to start", || runner.started().len() == 1).await;
    // The controller is holding `held`; mark it paused as a preemption would.
    e.ctx
        .store
        .set_status(&held.id, TaskStatus::Paused, None)
        .unwrap();

    let parked = e.ctx.store.create_task(&e.spec("parked"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&parked.id, TaskStatus::Paused, None)
        .unwrap();

    let reply = api::set_enabled(&e.ctx, true).unwrap();
    assert_eq!(reply.resumed, 1);
    assert_eq!(
        e.ctx.store.require_task(&parked.id).unwrap().status,
        TaskStatus::Queued
    );
    assert_eq!(
        e.ctx.store.require_task(&held.id).unwrap().status,
        TaskStatus::Paused
    );
    runner.release.add_permits(10);
    until("parked to run", || runner.started().contains(&parked.id)).await;
}

#[tokio::test]
async fn a_restart_requeues_what_was_running_or_queued() {
    let runner = FakeRunner::new(false, vec![]);
    let e = env(6, runner.clone(), AutonomyLevel::Full);
    let running = e.ctx.store.create_task(&e.spec("r"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&running.id, TaskStatus::Running, None)
        .unwrap();
    let queued = e.ctx.store.create_task(&e.spec("q"), "/p", 8).unwrap();
    let done = e.ctx.store.create_task(&e.spec("d"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&done.id, TaskStatus::Done, None)
        .unwrap();

    assert_eq!(api::resume_after_restart(&e.ctx).unwrap(), 2);
    until("both to run", || runner.started().len() == 2).await;
    let started = runner.started();
    assert!(started.contains(&running.id) && started.contains(&queued.id));
    assert!(!started.contains(&done.id));
}

#[tokio::test]
async fn a_restart_with_the_assistant_disabled_leaves_tasks_interrupted() {
    let runner = FakeRunner::new(false, vec![]);
    let e = env(6, runner.clone(), AutonomyLevel::Full);
    e.ctx.controller.set_enabled(false);
    let t = e.ctx.store.create_task(&e.spec("r"), "/p", 8).unwrap();
    e.ctx
        .store
        .set_status(&t.id, TaskStatus::Running, None)
        .unwrap();
    assert_eq!(api::resume_after_restart(&e.ctx).unwrap(), 0);
    assert_eq!(
        e.ctx.store.require_task(&t.id).unwrap().status,
        TaskStatus::Interrupted
    );
    assert!(runner.started().is_empty());
}

#[tokio::test]
async fn status_reports_the_step_effects_and_queue() {
    let e = env(4, FakeRunner::new(true, vec![]), AutonomyLevel::Full);
    let task = api::start_task(&e.ctx, e.spec("g")).unwrap();
    e.ctx.store.begin_step(&task.id, 1).unwrap();
    let detail = api::task_status(&e.ctx, &task.id).unwrap();
    assert_eq!(detail.step.unwrap().state, StepState::Started);
    assert_eq!(detail.queue.capacity, 4);
    assert!(detail.queue.enabled);
    assert!(matches!(
        api::task_status(&e.ctx, "nope"),
        Err(AssistantError::NotFound(_))
    ));
    assert_eq!(api::list_tasks(&e.ctx, Some(1000)).unwrap().len(), 1);
    e.runner.release.add_permits(10);
}

#[tokio::test]
async fn index_operations_go_through_the_same_root_checks() {
    let e = env(4, FakeRunner::new(false, vec![]), AutonomyLevel::Full);
    let stats = api::index_refresh(&e.ctx, e.project.path()).await.unwrap();
    assert_eq!(stats.changed, 1);
    assert_eq!(
        api::index_status(&e.ctx, e.project.path()).unwrap().files,
        1
    );
    let hits = api::search(&e.ctx, e.project.path(), "alpha_fn", Some(1000)).unwrap();
    assert_eq!(hits[0].path, "a.rs");
    assert!(hits.len() <= api::SEARCH_MAX);
    assert!(api::index_refresh(&e.ctx, &e.ctx.workspace).await.is_err());
    assert!(api::search(&e.ctx, &e.ctx.workspace, "x", None).is_err());
}
