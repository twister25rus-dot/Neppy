//! Per-turn Debug Mode scope: repo scoping plus automatic task + checkpoint.
//!
//! A chat turn on a Debug-mode thread is wrapped by [`run`]. Before the turn it
//! resolves PROJECT_ROOT (a failure here is fatal: the turn must never run
//! unscoped), starts a debug task and takes a checkpoint; around the turn it
//! scopes `turn_workspace::with_workspace(root, ..)` — which must enclose agent
//! *construction* too, since the session builder reads that scope — and the
//! [`DebugTurn`] task-local the `debug_checkpoint` / `debug_report` tools read;
//! after the turn it records the files changed and the final task status.
//!
//! Bookkeeping failures (task, checkpoint, diff) are logged and never abort the
//! turn. Non-debug turns never reach this module.

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::neppy::config::schema::debug_mode::DebugModeConfig;

use super::candidate;
use super::ops::{self, DebugCtx};
use super::policy;
use super::selfmod;
use super::types::{TaskPatch, TaskStatus};

const REQUEST_CLIP: usize = 500;

/// The agent every Debug-mode turn runs. Reachable only inside a debug turn:
/// see [`ensure_agent_allowed`].
pub const DEBUG_AGENT_ID: &str = "debug_agent";

tokio::task_local! {
    static DEBUG_TURN: DebugTurn;
}

/// The debug task a turn is bound to.
#[derive(Debug, Clone)]
pub struct DebugTurn {
    pub ctx: DebugCtx,
    pub root: PathBuf,
    /// `None` when `task_start` failed (logged); the tools then refuse.
    pub task_id: Option<String>,
    /// The automatic pre-turn checkpoint, when it could be taken (or `None`
    /// when `auto_checkpoint` is off).
    pub checkpoint_id: Option<String>,
    /// The `[debug_mode]` settings this turn runs under, read once at turn
    /// start so a mid-turn edit cannot loosen a turn already in flight.
    pub settings: DebugModeConfig,
}

/// The debug turn the current task is running, if any.
pub fn current() -> Option<DebugTurn> {
    DEBUG_TURN.try_with(|t| t.clone()).ok()
}

/// Refuses `debug_agent` unless the current task is inside a Debug-mode turn
/// (spec section 40: normal conversations must not gain source-modification
/// privileges). Every other agent id is allowed. Called from the two choke
/// points that turn a definition into a running agent: the top-level session
/// builder (`Agent::build_session_agent_inner`) and `run_subagent`.
pub fn ensure_agent_allowed(agent_id: &str) -> Result<(), String> {
    if agent_id == DEBUG_AGENT_ID && current().is_none() {
        log::warn!("[debug_mode] refused to run `{DEBUG_AGENT_ID}` outside a Debug-mode turn");
        return Err(format!(
            "agent '{DEBUG_AGENT_ID}' is only available inside a Debug-mode turn"
        ));
    }
    Ok(())
}

/// Runs `fut` with `turn` as the ambient debug turn.
pub async fn with_turn<F: Future>(turn: DebugTurn, fut: F) -> F::Output {
    DEBUG_TURN.scope(turn, Box::pin(fut)).await
}

/// How a turn ended, as far as the bookkeeping cares.
#[derive(Debug, Clone)]
pub enum Outcome {
    Completed,
    Errored(String),
    Cancelled,
}

fn clip(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

/// Resolves PROJECT_ROOT for a turn (`explicit` is for tests; production
/// passes `None`: env `NEPPY_DEBUG_PROJECT_ROOT`, then `debug_mode.project_root`,
/// then the crate manifest dir). The error is user-visible; there is no unscoped
/// fallback.
pub(crate) async fn resolve_root(explicit: Option<&str>) -> Result<PathBuf, String> {
    resolve_root_with(explicit, &DebugModeConfig::default()).await
}

pub(crate) async fn resolve_root_with(
    explicit: Option<&str>,
    settings: &DebugModeConfig,
) -> Result<PathBuf, String> {
    super::git::resolve_project_root_with(explicit, settings.project_root.as_deref())
        .await
        .map_err(|e| {
            log::warn!("[debug_mode] turn refused: project root unavailable: {e}");
            format!(
                "Debug mode cannot start: the project repository could not be resolved ({e}). \
             Set the project root in Debug mode settings (or NEPPY_DEBUG_PROJECT_ROOT) to the repository root and try again."
            )
        })
}

/// Records the task + pre-turn checkpoint for a turn in `root`. Infallible:
/// bookkeeping errors are logged.
#[cfg(test)]
pub(crate) async fn begin(workspace_dir: &Path, root: PathBuf, message: &str) -> DebugTurn {
    begin_with(workspace_dir, root, message, DebugModeConfig::default()).await
}

/// [`begin`] under explicit settings.
pub(crate) async fn begin_with(
    workspace_dir: &Path,
    root: PathBuf,
    message: &str,
    settings: DebugModeConfig,
) -> DebugTurn {
    let ctx = DebugCtx::new(workspace_dir).with_settings(&settings);
    let request = clip(message, REQUEST_CLIP);
    let request = if request.is_empty() {
        "(empty request)".to_string()
    } else {
        request
    };

    let task_id = match ops::task_start(&ctx, &request).await {
        Ok(o) => Some(o.value.id),
        Err(e) => {
            log::warn!("[debug_mode] task_start failed: {e}");
            None
        }
    };
    // From here a recorded task exists: if this future is dropped before the
    // turn is handed back, the task must not be left `Editing`.
    let mut guard = CancelGuard(task_id.clone().map(|id| DebugTurn {
        ctx: ctx.clone(),
        root: root.clone(),
        task_id: Some(id),
        checkpoint_id: None,
        settings: settings.clone(),
    }));
    let mut checkpoint_id = None;
    if task_id.is_some() && !settings.auto_checkpoint {
        log::info!("[debug_mode] auto_checkpoint=false: skipping the pre-turn checkpoint");
    } else if task_id.is_some() {
        let root_s = root.display().to_string();
        let desc = format!("before: {request}");
        match ops::checkpoint_create(&ctx, Some(&root_s), &desc, task_id.as_deref()).await {
            Ok(o) => checkpoint_id = Some(o.value.id),
            Err(e) => log::warn!("[debug_mode] pre-turn checkpoint failed: {e}"),
        }
    }
    if let Some(id) = &task_id {
        let patch = TaskPatch {
            status: Some(TaskStatus::Editing),
            checkpoint_id: checkpoint_id.clone(),
            ..Default::default()
        };
        if let Err(e) = ops::task_update(&ctx, id, patch).await {
            log::warn!("[debug_mode] task_update(begin) failed: {e}");
        }
    }
    log::info!(
        "[debug_mode] turn begin task={} checkpoint={}",
        task_id.as_deref().unwrap_or("-"),
        checkpoint_id.as_deref().unwrap_or("-")
    );
    guard.0 = None;
    DebugTurn {
        ctx,
        root,
        task_id,
        checkpoint_id,
        settings,
    }
}

#[cfg(test)]
tokio::task_local! {
    /// Test hook: when scoped, `finish` signals it has started and then parks
    /// forever, so a test can drop the turn mid-finish.
    pub(crate) static TEST_PAUSE_IN_FINISH: std::sync::Arc<tokio::sync::Notify>;
}

/// Sorted, unique paths changed in `root` since checkpoint `cp` (or since HEAD
/// when `cp` is `None`), including untracked files. `None` when the diff fails.
pub(super) async fn diff_files(
    ctx: &DebugCtx,
    root: &str,
    cp: Option<&str>,
) -> Option<Vec<String>> {
    match ops::diff(ctx, Some(root), cp).await {
        Ok(o) => {
            let d = o.value;
            let mut f: Vec<String> = d.files.into_iter().map(|f| f.path).collect();
            for u in d.untracked {
                if !f.contains(&u) {
                    f.push(u);
                }
            }
            f.sort();
            Some(f)
        }
        Err(e) => {
            log::warn!("[debug_mode] post-turn diff failed: {e}");
            None
        }
    }
}

/// Records the turn's result on its task. Never fails the caller; returns
/// whether the task now holds the final status (`true` also when there is no
/// task to update).
pub async fn finish(turn: &DebugTurn, outcome: Outcome) -> bool {
    #[cfg(test)]
    if let Ok(n) = TEST_PAUSE_IN_FINISH.try_with(|n| n.clone()) {
        n.notify_one();
        std::future::pending::<()>().await;
    }
    let Some(task_id) = turn.task_id.as_deref() else {
        return true;
    };
    let root_s = turn.root.display().to_string();
    // With no checkpoint (`auto_checkpoint = false`) the diff is against HEAD.
    let files = diff_files(&turn.ctx, &root_s, turn.checkpoint_id.as_deref()).await;
    let assessment = selfmod::assess(files.as_deref().unwrap_or_default());
    if assessment.critical {
        let crit = assessment.critical_files.clone();
        if let Err(e) = turn
            .ctx
            .store
            .task_modify(task_id, move |t| t.critical_files = crit)
        {
            log::warn!("[debug_mode] cannot record critical files: {e}");
        }
    }
    let reported = match ops::task_get(&turn.ctx, task_id).await {
        Ok(o) => Some(o.value),
        Err(e) => {
            log::warn!("[debug_mode] task_get(finish) failed: {e}");
            None
        }
    };
    let reported_status = reported.as_ref().map(|t| t.status);
    let has_summary = reported.as_ref().is_some_and(|t| t.summary.is_some());

    let (status, summary) = match &outcome {
        Outcome::Errored(e) => (
            TaskStatus::Failed,
            (!has_summary).then(|| format!("Turn failed: {}", clip(e, 300))),
        ),
        Outcome::Cancelled => (
            TaskStatus::Failed,
            (!has_summary).then(|| "Turn was cancelled before completion.".to_string()),
        ),
        Outcome::Completed => {
            let status = match reported_status {
                Some(s @ (TaskStatus::Pass | TaskStatus::Partial | TaskStatus::Failed)) => s,
                _ => match &files {
                    Some(f) if f.is_empty() => TaskStatus::Pass,
                    _ => TaskStatus::Partial,
                },
            };
            if status == TaskStatus::Pass
                && assessment.critical
                && !candidate::candidate_valid_for_current_tree(&turn.ctx, &turn.root).await
            {
                log::info!(
                    "[debug_mode] turn pass downgraded task={task_id} critical_files={}",
                    assessment.critical_files.len()
                );
                (TaskStatus::Partial, None)
            } else {
                (status, None)
            }
        }
    };
    let patch = TaskPatch {
        status: Some(status),
        files_changed: files.clone(),
        summary,
        ..Default::default()
    };
    match ops::task_update(&turn.ctx, task_id, patch).await {
        Ok(_) => {
            log::info!(
                "[debug_mode] turn end task={task_id} status={status:?} files_changed={}",
                files.map_or_else(|| "?".to_string(), |f| f.len().to_string())
            );
            true
        }
        Err(e) => {
            log::warn!("[debug_mode] task_update(finish) failed: {e}");
            false
        }
    }
}

/// Finalises a turn as cancelled if its future is dropped before the turn's
/// final status has been written (including mid-`finish`).
struct CancelGuard(Option<DebugTurn>);

impl Drop for CancelGuard {
    fn drop(&mut self) {
        let Some(turn) = self.0.take() else { return };
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn(async move { finish(&turn, Outcome::Cancelled).await });
            }
            Err(_) => log::warn!("[debug_mode] turn dropped without a runtime; task left active"),
        }
    }
}

/// Runs one Debug-mode turn. `fut` must perform the *whole* turn including
/// agent/session construction: it runs inside the repo scope
/// (`turn_workspace::with_workspace`) and the [`DebugTurn`] scope.
///
/// A disabled Debug Mode (`debug_mode.enabled = false`) refuses here, before
/// any task, checkpoint or agent exists.
pub async fn run<T, F>(workspace_dir: &Path, message: &str, fut: F) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    let settings = super::settings::load().await.map_err(|e| {
        log::warn!("[debug_mode] turn refused: settings unreadable: {e}");
        format!("Debug mode cannot start: its settings could not be read ({e}).")
    })?;
    policy::ensure_enabled(&settings)?;
    let root = resolve_root_with(None, &settings).await?;
    run_in_root_with(workspace_dir, root, message, settings, fut).await
}

/// [`run`] with the root already resolved, under default settings.
#[cfg(test)]
pub(crate) async fn run_in_root<T, F>(
    workspace_dir: &Path,
    root: PathBuf,
    message: &str,
    fut: F,
) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    run_in_root_with(
        workspace_dir,
        root,
        message,
        DebugModeConfig::default(),
        fut,
    )
    .await
}

/// [`run`] with the root and settings already resolved.
pub(crate) async fn run_in_root_with<T, F>(
    workspace_dir: &Path,
    root: PathBuf,
    message: &str,
    settings: DebugModeConfig,
    fut: F,
) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    policy::ensure_enabled(&settings)?;
    let extra_roots = policy::external_roots(&settings);
    let turn = begin_with(workspace_dir, root, message, settings).await;
    let mut guard = CancelGuard(Some(turn.clone()));
    let result = crate::neppy::agent::turn_workspace::with_workspace(
        turn.root.clone(),
        crate::neppy::agent::turn_workspace::with_extra_roots(
            extra_roots,
            with_turn(turn.clone(), fut),
        ),
    )
    .await;
    // The guard stays armed through `finish`: a drop during the post-turn
    // diff / task read / task update must still end the task `failed`.
    let finished = match &result {
        Ok(_) => finish(&turn, Outcome::Completed).await,
        Err(e) => finish(&turn, Outcome::Errored(e.clone())).await,
    };
    if finished {
        guard.0 = None;
    }
    result
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
