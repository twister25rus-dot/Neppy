//! The operations behind the `local_assistant.*` RPCs, separated from
//! transport and from the process-global controller so they can be driven with
//! a scripted controller in tests.
//!
//! Every path a caller names is resolved and checked here, once, before it
//! reaches the index or the step loop.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;

use crate::neppy::config::schema::LocalAssistantConfig;
use crate::neppy::security::SecurityPolicy;

use super::index::{open_index, IndexStatus, RefreshStats};
use super::ops::{Controller, QueueItem};
use super::store::StateStore;
use super::tests_runner::check_command;
use super::types::*;

/// Most tasks `list` returns, and its default.
const LIST_DEFAULT: usize = 20;
const LIST_MAX: usize = 100;
/// Snippets the debug `search` returns at most.
pub(crate) const SEARCH_MAX: usize = 20;
/// Effects shown with a task.
const EFFECTS_SHOWN: usize = 50;
/// Longest goal accepted, bytes.
const GOAL_LIMIT: usize = 4000;
const COMMAND_LIMIT: usize = 2000;

pub(crate) struct ApiCtx {
    pub(crate) cfg: LocalAssistantConfig,
    pub(crate) workspace: PathBuf,
    pub(crate) store: Arc<StateStore>,
    pub(crate) policy: Arc<SecurityPolicy>,
    pub(crate) controller: Arc<Controller>,
}

#[derive(Debug, Serialize)]
pub(crate) struct StepInfo {
    pub(crate) step_no: u32,
    pub(crate) state: StepState,
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
}

#[derive(Debug, Serialize)]
pub(crate) struct EffectInfo {
    pub(crate) step_no: u32,
    pub(crate) kind: EffectKind,
    pub(crate) path: String,
    pub(crate) status: EffectStatus,
    pub(crate) detail: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct QueueInfo {
    pub(crate) pending: usize,
    pub(crate) capacity: usize,
    pub(crate) enabled: bool,
    pub(crate) running_task: Option<TaskId>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskDetail {
    pub(crate) task: TaskRecord,
    pub(crate) step: Option<StepInfo>,
    pub(crate) effects: Vec<EffectInfo>,
    pub(crate) queue: QueueInfo,
}

#[derive(Debug, Serialize)]
pub(crate) struct EnabledReply {
    pub(crate) enabled: bool,
    /// Paused tasks put back in the queue.
    pub(crate) resumed: usize,
}

/// A directory the assistant may work on: it exists, is not a protected
/// location, and neither contains nor sits inside the Neppy workspace.
pub(crate) fn resolve_root(ctx: &ApiCtx, root: &Path) -> Result<PathBuf> {
    let canon = root
        .canonicalize()
        .map_err(|e| AssistantError::Invalid(format!("project_root `{}`: {e}", root.display())))?;
    if !canon.is_dir() {
        return Err(AssistantError::Invalid(
            "project_root is not a directory".into(),
        ));
    }
    if SecurityPolicy::is_always_forbidden(&canon) {
        return Err(AssistantError::Invalid(
            "project_root is a protected location".into(),
        ));
    }
    let ws = ctx
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| ctx.workspace.clone());
    if canon.starts_with(&ws) || ws.starts_with(&canon) {
        return Err(AssistantError::Invalid(
            "project_root must not contain, or be inside, the Neppy workspace".into(),
        ));
    }
    Ok(canon)
}

fn accepting(ctx: &ApiCtx) -> Result<()> {
    if !ctx.cfg.enabled || !ctx.controller.enabled() {
        return Err(AssistantError::Disabled);
    }
    Ok(())
}

pub(crate) fn start_task(ctx: &ApiCtx, spec: TaskSpec) -> Result<TaskRecord> {
    accepting(ctx)?;
    let goal = spec.goal.trim();
    if goal.is_empty() {
        return Err(AssistantError::Invalid("goal is empty".into()));
    }
    if goal.len() > GOAL_LIMIT {
        return Err(AssistantError::Invalid(format!(
            "goal is too long ({} bytes; limit {GOAL_LIMIT})",
            goal.len()
        )));
    }
    let root = resolve_root(ctx, &spec.project_root)?;
    if spec.allow_edits && !ctx.policy.can_act() {
        return Err(AssistantError::Invalid(
            "allow_edits was requested but the autonomy tier is read-only".into(),
        ));
    }
    let test_command = spec
        .test_command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if let Some(command) = test_command {
        if command.len() > COMMAND_LIMIT {
            return Err(AssistantError::Invalid("test_command is too long".into()));
        }
        check_command(&ctx.policy, command)
            .map_err(|why| AssistantError::Invalid(format!("test_command refused: {why}")))?;
    }
    let max_steps = spec
        .max_steps
        .map_or(ctx.cfg.max_steps, |n| n.clamp(1, ctx.cfg.max_steps));
    let spec = TaskSpec {
        test_command: test_command.map(str::to_string),
        ..spec
    };
    // Take a queue slot before writing anything: a refused task leaves no row.
    let slot = ctx.controller.reserve()?;
    let task = ctx
        .store
        .create_task(&spec, &root.to_string_lossy(), max_steps)?;
    log::info!(
        "[local_assistant] task {} accepted: root={} allow_edits={} test_command={} max_steps={max_steps}",
        task.id,
        root.display(),
        task.allow_edits,
        task.test_command.is_some()
    );
    slot.submit(QueueItem {
        workspace: ctx.workspace.clone(),
        task_id: task.id.clone(),
    })?;
    Ok(task)
}

pub(crate) fn task_status(ctx: &ApiCtx, id: &str) -> Result<TaskDetail> {
    let task = ctx.store.require_task(id)?;
    let latest = ctx.store.latest_step(id)?;
    let effects = match &latest {
        Some(step) => ctx
            .store
            .effects_of_step(id, step.step_no)?
            .into_iter()
            .take(EFFECTS_SHOWN)
            .map(|e| EffectInfo {
                step_no: e.step_no,
                kind: e.kind,
                path: e.path,
                status: e.status,
                detail: e.result.map(|r| clip(&r, 300)),
            })
            .collect(),
        None => Vec::new(),
    };
    Ok(TaskDetail {
        task,
        step: latest.map(|s| StepInfo {
            step_no: s.step_no,
            state: s.state,
            prompt_tokens: s.prompt_tokens,
            completion_tokens: s.completion_tokens,
        }),
        effects,
        queue: QueueInfo {
            pending: ctx.controller.pending(),
            capacity: ctx.controller.capacity(),
            enabled: ctx.controller.enabled(),
            running_task: ctx.controller.current_task(),
        },
    })
}

pub(crate) fn list_tasks(ctx: &ApiCtx, limit: Option<usize>) -> Result<Vec<TaskRecord>> {
    ctx.store
        .list_tasks(limit.unwrap_or(LIST_DEFAULT).clamp(1, LIST_MAX))
}

pub(crate) fn resume_task(ctx: &ApiCtx, id: &str) -> Result<TaskRecord> {
    accepting(ctx)?;
    let task = ctx.store.require_task(id)?;
    if !task.status.is_resumable() {
        return Err(AssistantError::Invalid(format!(
            "task is {}; only paused, interrupted or failed tasks can be resumed",
            task.status.as_str()
        )));
    }
    let slot = ctx.controller.reserve()?;
    let from = [task.status];
    if !ctx.store.transition(id, &from, TaskStatus::Queued)? {
        return Err(AssistantError::Invalid(
            "the task changed state; try again".into(),
        ));
    }
    if let Err(err) = slot.submit(QueueItem {
        workspace: ctx.workspace.clone(),
        task_id: id.to_string(),
    }) {
        ctx.store
            .set_status(id, task.status, task.error.as_deref())?;
        return Err(err);
    }
    log::info!(
        "[local_assistant] task {id} queued for resume from {}",
        task.status.as_str()
    );
    ctx.store.require_task(id)
}

pub(crate) fn cancel_task(ctx: &ApiCtx, id: &str) -> Result<TaskRecord> {
    let moved = ctx.store.transition(
        id,
        &[
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Interrupted,
        ],
        TaskStatus::Cancelled,
    )?;
    if moved {
        ctx.controller.cancel_current(id);
        log::info!("[local_assistant] task {id} cancel requested");
    }
    ctx.store.require_task(id)
}

pub(crate) fn set_enabled(ctx: &ApiCtx, enabled: bool) -> Result<EnabledReply> {
    ctx.controller.set_enabled(enabled);
    let mut resumed = 0;
    if enabled && ctx.cfg.enabled {
        for task in ctx.store.list_tasks(LIST_MAX)? {
            // A task the controller is still holding (waiting out memory
            // pressure) will be retried by it; queueing it again would run it
            // twice.
            if task.status != TaskStatus::Paused || ctx.controller.is_current(&task.id) {
                continue;
            }
            let Ok(slot) = ctx.controller.reserve() else {
                break;
            };
            if !ctx
                .store
                .transition(&task.id, &[TaskStatus::Paused], TaskStatus::Queued)?
            {
                continue;
            }
            if slot
                .submit(QueueItem {
                    workspace: ctx.workspace.clone(),
                    task_id: task.id.clone(),
                })
                .is_ok()
            {
                resumed += 1;
            } else {
                ctx.store.set_status(&task.id, TaskStatus::Paused, None)?;
            }
        }
    }
    log::info!("[local_assistant] enabled={enabled} resumed={resumed} paused task(s)");
    Ok(EnabledReply { enabled, resumed })
}

/// After a restart: whatever was running or queued is interrupted, and (when
/// enabled) goes back in the queue. Returns how many were re-queued.
pub(crate) fn resume_after_restart(ctx: &ApiCtx) -> Result<usize> {
    let interrupted = ctx.store.mark_interrupted_on_boot()?;
    if !ctx.cfg.enabled || !ctx.controller.enabled() {
        return Ok(0);
    }
    let mut queued = 0;
    for id in interrupted {
        let Ok(slot) = ctx.controller.reserve() else {
            log::warn!(
                "[local_assistant] queue full while resuming after restart; {id} stays interrupted"
            );
            break;
        };
        if !ctx
            .store
            .transition(&id, &[TaskStatus::Interrupted], TaskStatus::Queued)?
        {
            continue;
        }
        if slot
            .submit(QueueItem {
                workspace: ctx.workspace.clone(),
                task_id: id.clone(),
            })
            .is_ok()
        {
            queued += 1;
        } else {
            ctx.store.set_status(&id, TaskStatus::Interrupted, None)?;
        }
    }
    Ok(queued)
}

pub(crate) async fn index_refresh(ctx: &ApiCtx, root: &Path) -> Result<RefreshStats> {
    let root = resolve_root(ctx, root)?;
    let index = Arc::new(open_index(&ctx.workspace, &root)?);
    index.refresh_async(&ctx.cfg).await
}

pub(crate) fn index_status(ctx: &ApiCtx, root: &Path) -> Result<IndexStatus> {
    let root = resolve_root(ctx, root)?;
    open_index(&ctx.workspace, &root)?.status()
}

/// Debug search: what retrieval would return for `query`, capped at
/// [`SEARCH_MAX`] snippets.
pub(crate) fn search(
    ctx: &ApiCtx,
    root: &Path,
    query: &str,
    limit: Option<usize>,
) -> Result<Vec<Snippet>> {
    let root = resolve_root(ctx, root)?;
    let index = open_index(&ctx.workspace, &root)?;
    let terms = super::index::query_terms(&[query]);
    let mut cfg = ctx.cfg.clone();
    cfg.max_snippets = limit.unwrap_or(cfg.max_snippets).clamp(1, SEARCH_MAX);
    index.search(&terms, &cfg)
}
