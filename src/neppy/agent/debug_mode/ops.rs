//! Debug Mode business logic behind the `debug_mode` RPC namespace.
//!
//! Every public op appends one audit line (`{ts, op, target, outcome}`) —
//! never file contents, env values or command output.

use std::path::Path;
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

use crate::rpc::RpcOutcome;

use super::checkpoints;
use super::checks;
use super::exec::{self, Keep, RunSpec};
use super::git::{resolve_project_root_with, Git};
use super::secrets;
use super::store::DebugStore;
use super::types::*;

pub(super) type RpcResult<T> = Result<RpcOutcome<T>, String>;

/// Serialises checkpoint creation and rollback against each other.
pub(super) static REPO_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

const MAX_TEXT: usize = 4000;
const MAX_FILES: usize = 2000;
const MAX_VALIDATIONS: usize = 100;
pub(super) const STORED_OUTPUT_TAIL: usize = 4096;

/// Handle on the per-workspace Debug Mode state.
#[derive(Debug, Clone)]
pub struct DebugCtx {
    pub store: DebugStore,
    /// The saved `[debug_mode]` settings this ctx enforces (defaults for
    /// [`DebugCtx::new`]; production handlers use [`DebugCtx::with_settings`]).
    pub settings: crate::neppy::config::schema::debug_mode::DebugModeConfig,
}

impl DebugCtx {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            store: DebugStore::new(workspace_dir),
            settings: Default::default(),
        }
    }

    /// Binds the saved settings (project root, commit permission, ...).
    pub fn with_settings(
        mut self,
        cfg: &crate::neppy::config::schema::debug_mode::DebugModeConfig,
    ) -> Self {
        self.settings = cfg.clone();
        self
    }

    /// Param, then env, then configured root, then the build-time default.
    pub(super) async fn resolve_root(
        &self,
        project_root: Option<&str>,
    ) -> Result<std::path::PathBuf, String> {
        resolve_project_root_with(project_root, self.settings.project_root.as_deref()).await
    }
}

pub(super) fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

pub(super) fn audit<T>(ctx: &DebugCtx, op: &str, target: &str, result: &Result<T, String>) {
    let outcome = match result {
        Ok(_) => "ok".to_string(),
        Err(e) => error_outcome(e),
    };
    audit_raw(ctx, op, target, outcome);
}

fn error_outcome(e: &str) -> String {
    format!("error: {}", clip(e.lines().next().unwrap_or(""), 200))
}

fn audit_raw(ctx: &DebugCtx, op: &str, target: &str, outcome: String) {
    log::debug!("[debug_mode] op={op} target={target} outcome={outcome}");
    let entry = AuditEntry {
        ts: chrono::Utc::now().to_rfc3339(),
        op: op.to_string(),
        target: clip(target, 200),
        outcome,
    };
    if let Err(e) = ctx.store.audit_append(&entry) {
        log::warn!("[debug_mode] audit append failed: {e}");
    }
}

pub(super) fn done<T>(value: T) -> RpcResult<T> {
    Ok(RpcOutcome::new(value, vec![]))
}

pub(super) async fn git_for(ctx: &DebugCtx, project_root: Option<&str>) -> Result<Git, String> {
    Ok(Git::new(&ctx.resolve_root(project_root).await?))
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

// ── status / checks ─────────────────────────────────────────────────────

pub async fn status(ctx: &DebugCtx, project_root: Option<&str>) -> RpcResult<DebugStatus> {
    let r = async {
        let git = git_for(ctx, project_root).await?;
        let active = ctx.store.active_task()?;
        Ok(DebugStatus {
            project_root: git.root().display().to_string(),
            branch: git.branch().await?,
            head: git.head().await?,
            dirty: git.dirty().await?,
            task_active: active.is_some(),
            active_task_id: active.map(|t| t.id),
        })
    }
    .await;
    audit(ctx, "status", project_root.unwrap_or("-"), &r);
    r.and_then(done)
}

pub async fn discover_checks(
    ctx: &DebugCtx,
    project_root: Option<&str>,
) -> RpcResult<Vec<DebugCheck>> {
    let r = async {
        let root = ctx.resolve_root(project_root).await?;
        Ok(checks::discover(&root))
    }
    .await;
    audit(ctx, "discover_checks", project_root.unwrap_or("-"), &r);
    r.and_then(done)
}

// ── checkpoints ─────────────────────────────────────────────────────────

pub async fn checkpoint_create(
    ctx: &DebugCtx,
    project_root: Option<&str>,
    description: &str,
    task_id: Option<&str>,
) -> RpcResult<Checkpoint> {
    let r = async {
        if description.trim().is_empty() {
            return Err("description must not be empty".to_string());
        }
        let git = git_for(ctx, project_root).await?;
        let _g = REPO_LOCK.lock().await;
        checkpoints::create(&ctx.store, &git, description, task_id).await
    }
    .await;
    let target = r
        .as_ref()
        .map(|c| c.id.clone())
        .unwrap_or_else(|_| "-".into());
    audit(ctx, "checkpoint_create", &target, &r);
    r.and_then(done)
}

pub async fn checkpoint_list(ctx: &DebugCtx, limit: Option<u64>) -> RpcResult<Vec<Checkpoint>> {
    let r = ctx
        .store
        .checkpoint_list(limit.unwrap_or(50).clamp(1, 500) as usize);
    audit(ctx, "checkpoint_list", "-", &r);
    r.and_then(done)
}

pub async fn checkpoint_get(ctx: &DebugCtx, checkpoint_id: &str) -> RpcResult<Checkpoint> {
    let r = ctx
        .store
        .checkpoint_get(checkpoint_id)
        .and_then(|c| c.ok_or_else(|| format!("unknown checkpoint '{checkpoint_id}'")));
    audit(ctx, "checkpoint_get", checkpoint_id, &r);
    r.and_then(done)
}

pub async fn rollback(
    ctx: &DebugCtx,
    project_root: Option<&str>,
    checkpoint_id: &str,
    confirm: bool,
) -> RpcResult<RollbackResult> {
    let r = async {
        if !confirm {
            return Err("rollback refused: pass confirm=true to proceed".to_string());
        }
        let cp = ctx
            .store
            .checkpoint_get(checkpoint_id)?
            .ok_or_else(|| format!("unknown checkpoint '{checkpoint_id}'"))?;
        let git = git_for(ctx, project_root.or(Some(cp.project_root.as_str()))).await?;
        let result = {
            let _g = REPO_LOCK.lock().await;
            checkpoints::rollback(&ctx.store, &git, &cp).await?
        };
        if let Some(task_id) = &cp.task_id {
            // The rollback already happened; a history hiccup must not turn it
            // into an error. Log and audit it as a warning instead.
            if let Err(e) = ctx
                .store
                .task_modify(task_id, |t| t.status = TaskStatus::RolledBack)
            {
                log::warn!("[debug_mode] rollback done but task update failed: {e}");
                audit_raw(
                    ctx,
                    "rollback_task_update",
                    task_id,
                    format!("warning: {}", clip(e.lines().next().unwrap_or(""), 200)),
                );
            }
        }
        Ok(result)
    }
    .await;
    audit(ctx, "rollback", checkpoint_id, &r);
    r.and_then(done)
}

pub async fn diff(
    ctx: &DebugCtx,
    project_root: Option<&str>,
    checkpoint_id: Option<&str>,
) -> RpcResult<DiffResult> {
    let r = async {
        let cp = match checkpoint_id {
            Some(id) => Some(
                ctx.store
                    .checkpoint_get(id)?
                    .ok_or_else(|| format!("unknown checkpoint '{id}'"))?,
            ),
            None => None,
        };
        let git = git_for(
            ctx,
            project_root.or(cp.as_ref().map(|c| c.project_root.as_str())),
        )
        .await?;
        let mut d = checkpoints::diff(&git, cp.as_ref()).await?;
        // Secret files never leave the core as diff hunks (spec section 22).
        d.text = secrets::mask_diff(&d.text);
        Ok(d)
    }
    .await;
    audit(ctx, "diff", checkpoint_id.unwrap_or("HEAD"), &r);
    r.and_then(done)
}

// ── task history ────────────────────────────────────────────────────────

pub async fn task_start(ctx: &DebugCtx, request: &str) -> RpcResult<TaskRecord> {
    let r = (|| {
        let request = request.trim();
        if request.is_empty() {
            return Err("request must not be empty".to_string());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let task = TaskRecord {
            id: new_id("task"),
            request: clip(request, MAX_TEXT),
            created_at: now.clone(),
            updated_at: now,
            status: TaskStatus::Planning,
            files_changed: vec![],
            validation: vec![],
            summary: None,
            checkpoint_id: None,
            branch: None,
            commit: None,
            critical_files: vec![],
            candidate_id: None,
        };
        ctx.store.task_add(task.clone())?;
        // A new task makes every older still-active one stale: close them so
        // they can never resurface as "the active task".
        match ctx.store.abandon_active_except(&task.id) {
            Ok(closed) if closed.is_empty() => {
                log::debug!(
                    "[debug_mode] task_start {}: no stale tasks to close",
                    task.id
                );
            }
            Ok(closed) => log::info!(
                "[debug_mode] task_start {}: closed {} stale task(s): {}",
                task.id,
                closed.len(),
                closed.join(", ")
            ),
            // The new task exists; a failure to tidy the old ones must not
            // fail the turn. `active_task` is newest-only regardless.
            Err(e) => log::warn!(
                "[debug_mode] task_start {}: cannot close stale tasks: {e}",
                task.id
            ),
        }
        Ok(task)
    })();
    let target = r
        .as_ref()
        .map(|t| t.id.clone())
        .unwrap_or_else(|_| "-".into());
    audit(ctx, "task_start", &target, &r);
    r.and_then(done)
}

pub async fn task_update(ctx: &DebugCtx, task_id: &str, patch: TaskPatch) -> RpcResult<TaskRecord> {
    let r = (|| {
        if let Some(id) = &patch.checkpoint_id {
            if ctx.store.checkpoint_get(id)?.is_none() {
                return Err(format!("unknown checkpoint '{id}'"));
            }
        }
        let mut patch = patch.clone();
        if let Some(files) = patch.files_changed.as_mut() {
            files.truncate(MAX_FILES);
        }
        if let Some(v) = patch.validation.as_mut() {
            v.truncate(MAX_VALIDATIONS);
            for rec in v.iter_mut() {
                rec.output_tail =
                    secrets::mask_secret_like_lines(&clip(&rec.output_tail, STORED_OUTPUT_TAIL));
            }
        }
        ctx.store
            .task_modify(task_id, move |t| {
                if let Some(s) = patch.status {
                    t.status = s;
                }
                if let Some(f) = patch.files_changed {
                    t.files_changed = f;
                }
                if let Some(v) = patch.validation {
                    t.validation = v;
                }
                if let Some(s) = patch.summary {
                    t.summary = Some(secrets::mask_secret_like_lines(&clip(&s, MAX_TEXT)));
                }
                if patch.checkpoint_id.is_some() {
                    t.checkpoint_id = patch.checkpoint_id;
                }
                if patch.branch.is_some() {
                    t.branch = patch.branch;
                }
                if patch.commit.is_some() {
                    t.commit = patch.commit;
                }
            })?
            .ok_or_else(|| format!("unknown task '{task_id}'"))
    })();
    audit(ctx, "task_update", task_id, &r);
    r.and_then(done)
}

pub async fn task_list(ctx: &DebugCtx, limit: Option<u64>) -> RpcResult<Vec<TaskRecord>> {
    let r = ctx
        .store
        .task_list(limit.unwrap_or(50).clamp(1, 500) as usize);
    audit(ctx, "task_list", "-", &r);
    r.and_then(done)
}

pub async fn task_get(ctx: &DebugCtx, task_id: &str) -> RpcResult<TaskRecord> {
    let r = ctx
        .store
        .task_get(task_id)
        .and_then(|t| t.ok_or_else(|| format!("unknown task '{task_id}'")));
    audit(ctx, "task_get", task_id, &r);
    r.and_then(done)
}

// ── run_check ───────────────────────────────────────────────────────────

fn tail_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn audit_target_for(check_id: Option<&str>, argv: &[String]) -> String {
    match check_id {
        Some(id) => id.to_string(),
        None => argv
            .iter()
            .take(3)
            .map(|a| clip(a, 64))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

pub async fn run_check(
    ctx: &DebugCtx,
    project_root: Option<&str>,
    check_id: Option<&str>,
    command: Option<Vec<String>>,
    timeout_secs: Option<u64>,
    task_id: Option<&str>,
) -> RpcResult<CheckResult> {
    let mut target = check_id.unwrap_or("-").to_string();
    let r = async {
        let root = ctx.resolve_root(project_root).await?;
        let discovered = checks::discover(&root);
        let (resolved_id, argv) = match (check_id, command) {
            (Some(id), None) => {
                let c = discovered
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or_else(|| format!("unknown check '{id}' (see discover_checks)"))?;
                (Some(c.id.clone()), c.command.clone())
            }
            (None, Some(cmd)) => (None, cmd),
            _ => return Err("provide exactly one of check_id or command".to_string()),
        };
        target = audit_target_for(resolved_id.as_deref(), &argv);
        checks::validate_command(&root, &argv, &discovered)?;
        if let Some(tid) = task_id {
            if ctx.store.task_get(tid)?.is_none() {
                return Err(format!("unknown task '{tid}'"));
            }
        }
        let timeout = Duration::from_secs(
            timeout_secs
                .unwrap_or(DEFAULT_CHECK_TIMEOUT_SECS)
                .clamp(1, MAX_CHECK_TIMEOUT_SECS),
        );
        log::debug!(
            "[debug_mode] run_check start target={target} timeout_s={}",
            timeout.as_secs()
        );
        let out = exec::run(RunSpec {
            program: &argv[0],
            args: &argv[1..],
            cwd: &root,
            timeout,
            cap: OUTPUT_TAIL_CAP,
            keep: Keep::Tail,
            env: &[],
        })
        .await?;
        let result = CheckResult {
            check_id: resolved_id,
            command: argv,
            exit_code: out.exit_code,
            passed: out.success(),
            timed_out: out.timed_out,
            duration_ms: out.duration.as_millis() as u64,
            stdout_tail: tail_text(&out.stdout),
            stderr_tail: tail_text(&out.stderr),
            stdout_truncated: out.stdout_truncated,
            stderr_truncated: out.stderr_truncated,
        };
        if let Some(tid) = task_id {
            let combined = format!("{}\n{}", result.stdout_tail, result.stderr_tail);
            let skip = combined.chars().count().saturating_sub(STORED_OUTPUT_TAIL);
            let rec = ValidationRecord {
                check_id: result.check_id.clone(),
                command: result.command.clone(),
                exit_code: result.exit_code,
                passed: result.passed,
                timed_out: result.timed_out,
                duration_ms: result.duration_ms,
                at: chrono::Utc::now().to_rfc3339(),
                output_tail: secrets::mask_secret_like_lines(
                    &combined.chars().skip(skip).collect::<String>(),
                ),
            };
            ctx.store.task_modify(tid, |t| {
                t.validation.push(rec);
                // Keep the NEWEST records: the pass gate needs the latest runs.
                if t.validation.len() > MAX_VALIDATIONS {
                    let cut = t.validation.len() - MAX_VALIDATIONS;
                    t.validation.drain(..cut);
                }
            })?;
            log::debug!(
                "[debug_mode] run_check recorded task={tid} passed={} exit={:?}",
                result.passed,
                result.exit_code
            );
        }
        Ok(result)
    }
    .await;
    // A failing check is a successful op; only policy/spawn errors are errors.
    let outcome = match &r {
        Ok(c) if c.passed => "pass".to_string(),
        Ok(c) if c.timed_out => "timeout".to_string(),
        Ok(c) => format!("fail exit={:?}", c.exit_code),
        Err(e) => error_outcome(e),
    };
    audit_raw(ctx, "run_check", &target, outcome);
    r.and_then(done)
}

pub async fn audit_tail(ctx: &DebugCtx, limit: Option<u64>) -> RpcResult<Vec<AuditEntry>> {
    let r = ctx
        .store
        .audit_tail(limit.unwrap_or(100).clamp(1, 1000) as usize);
    r.and_then(done)
}

#[cfg(test)]
#[path = "ops_run_tests.rs"]
mod ops_run_tests;
#[cfg(test)]
#[path = "ops_tests.rs"]
mod ops_tests;
