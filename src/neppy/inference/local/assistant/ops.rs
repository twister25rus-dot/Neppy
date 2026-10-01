//! The controller: a bounded queue and one task at a time.
//!
//! It is a single tokio task, started on first use, that takes task ids off a
//! bounded channel and runs each to completion, pause or cancellation. A task
//! whose model call was preempted by memory pressure keeps its place: the
//! controller waits for the pressure to clear and runs it again, and the step
//! state machine picks up exactly where it stopped.
//!
//! The controller is deliberately small. The model it uses is started lazily
//! by the gated model (see `gated_model`), so an idle controller holds no
//! model memory at all.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::neppy::config::rpc as config_rpc;
use crate::neppy::config::Config;
use crate::neppy::security::SecurityPolicy;

use super::super::service::mlx_admin::gate::GateError;
use super::api::{self, ApiCtx};
use super::faults::NoFaults;
use super::model::build_models;
use super::runner::{run_task, RunEnv, RunOutcome, StopSignal};
use super::store::StateStore;
use super::types::*;

/// Times one task is re-run after a preemption before it is left
/// `interrupted` for a manual resume.
const MAX_REQUEUES: u32 = 10;
/// Poll interval, and the longest wait, for memory pressure to clear.
const RESUME_POLL: Duration = Duration::from_secs(2);
const RESUME_MAX_WAIT: Duration = Duration::from_secs(300);
/// Delay before an in-place model retry.
const RETRY_DELAY: Duration = Duration::from_secs(5);
/// Workspaces whose state stores are kept open at once.
const STORES_OPEN: usize = 4;
/// How long a disable waits for the running step before stopping the worker.
const STOP_WORKER_WAIT: Duration = Duration::from_secs(120);

/// Whether a preemption reason is a yield to an interactive request.
fn is_yield(why: &str) -> bool {
    why == format!("{:?}", GateError::Yielded)
}

pub(crate) struct QueueItem {
    pub(crate) workspace: PathBuf,
    pub(crate) task_id: TaskId,
}

#[async_trait]
pub(crate) trait TaskRunner: Send + Sync {
    async fn run(&self, item: &QueueItem, stop: &StopSignal) -> Result<RunOutcome>;
    /// Wait until a preempted task can be tried again. `false` gives up.
    async fn wait_for_resume(&self, stop: &StopSignal) -> bool;
    /// Called when a task has been preempted too many times.
    async fn give_up(&self, item: &QueueItem, why: &str);
}

pub(crate) struct Controller {
    tx: mpsc::Sender<QueueItem>,
    /// Queued plus running.
    pending: AtomicUsize,
    cap: usize,
    /// Ids queued or running. A task is in the queue at most once, however
    /// many paths (accept, resume, the boot pass) try to put it there.
    tracked: Mutex<HashSet<TaskId>>,
    enabled: Arc<AtomicBool>,
    current: Mutex<Option<(TaskId, CancellationToken)>>,
    runner: Arc<dyn TaskRunner>,
}

/// A reserved place in the queue. Dropping it unused gives the place back.
pub(crate) struct Slot<'a> {
    ctl: &'a Controller,
    used: bool,
}

impl Slot<'_> {
    /// Queue `item`. A task already queued or running is not queued again:
    /// the place is given back and the call succeeds, because the task *is*
    /// in the queue, which is what the caller wanted.
    pub(crate) fn submit(mut self, item: QueueItem) -> Result<()> {
        self.used = true;
        if !self.ctl.tracked.lock().insert(item.task_id.clone()) {
            self.ctl.pending.fetch_sub(1, Ordering::SeqCst);
            log::info!(
                "[local_assistant] task {} is already queued or running; not queued again",
                item.task_id
            );
            return Ok(());
        }
        let id = item.task_id.clone();
        if self.ctl.tx.try_send(item).is_err() {
            self.ctl.tracked.lock().remove(&id);
            self.ctl.pending.fetch_sub(1, Ordering::SeqCst);
            return Err(AssistantError::QueueFull(self.ctl.cap));
        }
        Ok(())
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if !self.used {
            self.ctl.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Controller {
    /// Start the controller loop on the current tokio runtime.
    pub(crate) fn spawn(cap: usize, enabled: bool, runner: Arc<dyn TaskRunner>) -> Arc<Self> {
        let cap = cap.max(1);
        let (tx, rx) = mpsc::channel(cap);
        let ctl = Arc::new(Self {
            tx,
            pending: AtomicUsize::new(0),
            cap,
            tracked: Mutex::new(HashSet::new()),
            enabled: Arc::new(AtomicBool::new(enabled)),
            current: Mutex::new(None),
            runner,
        });
        tokio::spawn(Arc::clone(&ctl).run_loop(rx));
        log::info!("[local_assistant] controller started (queue cap {cap}, enabled={enabled})");
        ctl
    }

    /// Take a place in the queue, or refuse: `cap` tasks may be pending.
    pub(crate) fn reserve(&self) -> Result<Slot<'_>> {
        let before = self.pending.fetch_add(1, Ordering::SeqCst);
        if before >= self.cap {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            log::warn!("[local_assistant] queue full ({before}/{})", self.cap);
            return Err(AssistantError::QueueFull(before));
        }
        Ok(Slot {
            ctl: self,
            used: false,
        })
    }

    /// Ids of the tasks queued or running right now.
    pub(crate) fn tracked_ids(&self) -> Vec<TaskId> {
        self.tracked.lock().iter().cloned().collect()
    }

    pub(crate) fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    pub(crate) fn capacity(&self) -> usize {
        self.cap
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    pub(crate) fn current_task(&self) -> Option<TaskId> {
        self.current.lock().as_ref().map(|(id, _)| id.clone())
    }

    pub(crate) fn is_current(&self, id: &str) -> bool {
        self.current
            .lock()
            .as_ref()
            .is_some_and(|(cur, _)| cur == id)
    }

    /// Cancel the running task if it is `id`. Aborts an in-flight model call.
    pub(crate) fn cancel_current(&self, id: &str) -> bool {
        match self.current.lock().as_ref() {
            Some((cur, token)) if cur == id => {
                token.cancel();
                true
            }
            _ => false,
        }
    }

    async fn run_loop(self: Arc<Self>, mut rx: mpsc::Receiver<QueueItem>) {
        while let Some(item) = rx.recv().await {
            let cancel = CancellationToken::new();
            *self.current.lock() = Some((item.task_id.clone(), cancel.clone()));
            let stop = StopSignal {
                enabled: Arc::clone(&self.enabled),
                cancel,
            };
            log::info!("[local_assistant] task {} dequeued", item.task_id);
            let mut requeues = 0u32;
            loop {
                match self.runner.run(&item, &stop).await {
                    Ok(RunOutcome::Preempted(why)) => {
                        // Stepping aside for an interactive request is not memory
                        // pressure and says nothing about whether the task is
                        // healthy, so it does not use up the requeue budget.
                        if !is_yield(&why) {
                            requeues += 1;
                        }
                        if requeues > MAX_REQUEUES {
                            log::warn!(
                                "[local_assistant] task {} preempted {MAX_REQUEUES} times; leaving it interrupted",
                                item.task_id
                            );
                            self.runner.give_up(&item, &why).await;
                            break;
                        }
                        log::info!(
                            "[local_assistant] task {} preempted ({why}); waiting to resume ({requeues}/{MAX_REQUEUES})",
                            item.task_id
                        );
                        if !self.runner.wait_for_resume(&stop).await {
                            self.runner
                                .give_up(&item, "gave up waiting for memory pressure to clear")
                                .await;
                            break;
                        }
                    }
                    Ok(outcome) => {
                        log::info!(
                            "[local_assistant] task {} stopped: {outcome:?}",
                            item.task_id
                        );
                        break;
                    }
                    Err(err) => {
                        log::warn!("[local_assistant] task {} run error: {err}", item.task_id);
                        break;
                    }
                }
            }
            *self.current.lock() = None;
            self.tracked.lock().remove(&item.task_id);
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

// ---- production wiring --------------------------------------------------

static CONTROLLER: OnceLock<Arc<Controller>> = OnceLock::new();
static STORES: Mutex<Vec<(PathBuf, Arc<StateStore>)>> = Mutex::new(Vec::new());

/// The state store for `workspace`, opened once and kept. A desktop session
/// moves workspace on login and logout, so a few are kept, not one.
pub(crate) fn store_for(workspace: &std::path::Path) -> Result<Arc<StateStore>> {
    let mut stores = STORES.lock();
    if let Some((_, store)) = stores.iter().find(|(path, _)| path == workspace) {
        return Ok(Arc::clone(store));
    }
    let store = Arc::new(StateStore::open(workspace)?);
    if stores.len() >= STORES_OPEN {
        stores.remove(0);
    }
    stores.push((workspace.to_path_buf(), Arc::clone(&store)));
    Ok(store)
}

fn policy_for(config: &Config) -> Arc<SecurityPolicy> {
    crate::neppy::security::live_policy::current().unwrap_or_else(|| {
        Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        ))
    })
}

struct ProdRunner;

#[async_trait]
impl TaskRunner for ProdRunner {
    async fn run(&self, item: &QueueItem, stop: &StopSignal) -> Result<RunOutcome> {
        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(AssistantError::Io)?;
        let store = store_for(&item.workspace)?;
        let (model, fallback) = match build_models(&config) {
            Ok(models) => models,
            Err(why) => {
                store.set_status(&item.task_id, TaskStatus::Failed, Some(&why))?;
                return Ok(RunOutcome::Finished(TaskStatus::Failed));
            }
        };
        let service = super::super::global(&config);
        let env = RunEnv {
            store,
            workspace: item.workspace.clone(),
            cfg: config.local_assistant.clone(),
            policy: policy_for(&config),
            model,
            fallback,
            faults: Arc::new(NoFaults),
            metrics: Some(Arc::clone(&service.metrics)),
            retry_delay: RETRY_DELAY,
        };
        run_task(&env, &item.task_id, stop).await
    }

    async fn wait_for_resume(&self, stop: &StopSignal) -> bool {
        let Ok(config) = config_rpc::load_config_with_timeout().await else {
            return false;
        };
        let service = super::super::global(&config);
        let deadline = Instant::now() + RESUME_MAX_WAIT;
        loop {
            tokio::select! {
                _ = stop.cancel.cancelled() => return false,
                _ = tokio::time::sleep(RESUME_POLL) => {}
            }
            if service.gate.paused().is_none() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
        }
    }

    async fn give_up(&self, item: &QueueItem, why: &str) {
        if let Ok(store) = store_for(&item.workspace) {
            let _ = store.set_status(&item.task_id, TaskStatus::Interrupted, Some(why));
        }
    }
}

fn controller(config: &Config) -> Arc<Controller> {
    Arc::clone(CONTROLLER.get_or_init(|| {
        Controller::spawn(
            config.local_assistant.task_queue_cap,
            config.local_assistant.enabled,
            Arc::new(ProdRunner),
        )
    }))
}

/// Context for one RPC call, from the current config. Starts the controller
/// on first use.
pub(crate) async fn prod_ctx() -> std::result::Result<ApiCtx, String> {
    let config = config_rpc::load_config_with_timeout().await?;
    let store = store_for(&config.workspace_dir).map_err(|e| e.to_string())?;
    let ctl = controller(&config);
    let _ = store.prune(
        config.local_assistant.keep_tasks,
        config.local_assistant.keep_days,
        now_ms(),
    );
    Ok(ApiCtx {
        cfg: config.local_assistant.clone(),
        workspace: config.workspace_dir.clone(),
        store,
        policy: policy_for(&config),
        controller: ctl,
    })
}

/// Re-queue whatever a previous process left running or queued. Safe to call
/// at start-up; returns how many tasks went back in the queue.
pub async fn resume_interrupted_on_boot() -> usize {
    let ctx = match prod_ctx().await {
        Ok(ctx) => ctx,
        Err(err) => {
            log::warn!("[local_assistant] boot resume skipped: {err}");
            return 0;
        }
    };
    match api::resume_after_restart(&ctx) {
        Ok(n) => {
            log::info!("[local_assistant] boot: {n} interrupted task(s) re-queued");
            n
        }
        Err(err) => {
            log::warn!("[local_assistant] boot resume failed: {err}");
            0
        }
    }
}

/// After a disable: wait for the running step to reach its checkpoint, then
/// stop the worker so the model's memory is returned.
///
/// Only a worker this process started is stopped (`stop_if_held`: no spawn
/// marker fallback, so a server the user or another core started is left
/// alone), and only while no request holds the inference gate. A chat request
/// that arrived since the disable keeps the worker; the idle policy stops it
/// later.
pub(crate) fn stop_worker_when_idle(ctl: Arc<Controller>) {
    tokio::spawn(async move {
        let deadline = Instant::now() + STOP_WORKER_WAIT;
        while ctl.current_task().is_some() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        if ctl.enabled() || ctl.current_task().is_some() {
            return;
        }
        let Ok(config) = config_rpc::load_config_with_timeout().await else {
            return;
        };
        let service = super::super::global(&config);
        if let Some(id) = super::super::service::mlx_admin::worker::worker_server_id(&config) {
            let Some(_gate) = service.gate.try_acquire() else {
                log::info!(
                    "[local_assistant] disabled: worker `{id}` kept, the inference gate is busy"
                );
                return;
            };
            let stopped = service.mlx.stop_if_held(&config, &id).await;
            log::info!("[local_assistant] disabled: worker `{id}` stopped={stopped}");
        }
    });
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
