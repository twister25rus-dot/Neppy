//! "Build & install locally": build the modified source into a real `.app`
//! and swap it into `/Applications` without touching GitHub or the updater.
//!
//! Two RPC-driven stages with a deliberate gap between them:
//!
//! 1. `build` runs `tauri build --bundles app` in the background into a
//!    private target dir. The running app is never touched.
//! 2. `apply` (explicit `confirm`) starts `scripts/neppy-install-local.sh`
//!    fully detached and returns; the UI then quits the app. The script waits
//!    for the app to exit, backs up the installed copy, swaps the new one in,
//!    relaunches and rolls back if the launch marker never clears.
//!
//! State lives in `{workspace}/debug_mode/local-install.json`. The installer's
//! verdict is `last-local-install.json` in `{data dir}/debug_mode/`, written by
//! the script, which outlives this process.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::candidate;
use super::candidate_steps::tail;
use super::local_install_steps::{
    log_tail, HelperSpawner, InstallSteps, ShellSpawner, TauriSteps, BUILD_TIMEOUT, HELPER_SCRIPT,
    LOG_FILE, TARGET_DIR,
};
use super::ops::{self, done, DebugCtx, RpcResult};
use super::selfmod;
use super::turn;

const FILE: &str = "local-install.json";
pub(super) const RESULT_FILE: &str = "last-local-install.json";
const SEEN_FILE: &str = "last-local-install.seen";
const ERROR_CAP: usize = 8 * 1024;
const LOG_TAIL_BYTES: u64 = 4 * 1024;

static FILE_LOCK: Mutex<()> = Mutex::new(());

struct Slot {
    dir: PathBuf,
    handle: JoinHandle<()>,
}

/// Live background builds, keyed by store dir (one per workspace).
static RUNNING: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn is_live(dir: &Path) -> bool {
    let mut slots = lock(&RUNNING);
    slots.retain(|s| !s.handle.is_finished());
    slots.iter().any(|s| s.dir == dir)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalInstallPhase {
    /// Nothing built (or the install finished and the app restarted).
    #[default]
    Idle,
    Building,
    Ready,
    Failed,
    Installing,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalInstallRecord {
    pub phase: LocalInstallPhase,
    /// Version baked into the new bundle (`app/package.json` at build time).
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub bundle_path: Option<String>,
    #[serde(default)]
    pub error: String,
    /// Pid of the app that started the install; a different pid on a later
    /// status call means the app restarted and the install is over.
    #[serde(default)]
    pub app_pid: Option<u32>,
    /// When `apply` handed over to the helper.
    #[serde(default)]
    pub installing_since: Option<String>,
    /// Tail of the build output. Filled in per status call, never persisted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub log_tail: String,
}

/// What the installer script reports about its last run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalInstallResult {
    /// `installed` | `restored` | `failed`.
    pub status: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub backup: String,
    #[serde(default)]
    pub ts: String,
    #[serde(default)]
    pub reason: String,
    /// True once the UI acknowledged this result (one-time notice).
    #[serde(default)]
    pub seen: bool,
}

#[derive(Debug, Clone)]
struct InstallStore {
    dir: PathBuf,
}

impl InstallStore {
    fn read(&self) -> Result<LocalInstallRecord, String> {
        let path = self.dir.join(FILE);
        match fs::read(&path) {
            Ok(b) => Ok(serde_json::from_slice(&b).unwrap_or_else(|e| {
                log::warn!("[debug_mode] {FILE} unreadable ({e}); treating as idle");
                LocalInstallRecord::default()
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(format!("[debug_mode] cannot read {FILE}: {e}")),
        }
    }

    fn write(&self, rec: &LocalInstallRecord) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("[debug_mode] create dir: {e}"))?;
        let tmp = self
            .dir
            .join(format!(".{FILE}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let mut persisted = rec.clone();
        persisted.log_tail.clear();
        let data = serde_json::to_vec_pretty(&persisted).map_err(|e| e.to_string())?;
        let go = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            fs::rename(&tmp, self.dir.join(FILE))
        };
        go().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("[debug_mode] write {FILE}: {e}")
        })
    }

    fn modify<F: FnOnce(&mut LocalInstallRecord)>(
        &self,
        f: F,
    ) -> Result<LocalInstallRecord, String> {
        let _g = lock(&FILE_LOCK);
        let mut rec = self.read()?;
        f(&mut rec);
        self.write(&rec)?;
        Ok(rec)
    }

    /// A build with no live pipeline can never finish; an install whose app
    /// has since restarted is over (the verdict is in the result file).
    fn reconcile(&self, my_pid: u32) -> Result<(), String> {
        let rec = self.read()?;
        match rec.phase {
            LocalInstallPhase::Building if !is_live(&self.dir) => {
                log::warn!("[debug_mode] local install build was interrupted");
                self.modify(|r| {
                    r.phase = LocalInstallPhase::Failed;
                    r.finished_at = Some(chrono::Utc::now().to_rfc3339());
                    r.error = "interrupted: the build is no longer running".into();
                })?;
            }
            LocalInstallPhase::Installing if rec.app_pid != Some(my_pid) => {
                log::info!("[debug_mode] local install finished (app restarted); back to idle");
                self.modify(|r| *r = LocalInstallRecord::default())?;
            }
            LocalInstallPhase::Installing if install_overdue(&rec) => {
                // The app is still the one that asked: the helper gave up
                // waiting for it to quit (or never ran). The build is intact.
                log::warn!("[debug_mode] local install never took over; back to ready");
                self.modify(|r| {
                    r.phase = LocalInstallPhase::Ready;
                    r.app_pid = None;
                    r.installing_since = None;
                })?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// How long the app may stay alive after asking to be replaced: the helper
/// waits 60 s for it to exit, so past this it has already given up.
const INSTALL_GRACE: chrono::Duration = chrono::Duration::seconds(150);

fn install_overdue(rec: &LocalInstallRecord) -> bool {
    rec.installing_since
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|t| chrono::Utc::now().signed_duration_since(t) > INSTALL_GRACE)
}

/// Refuses when the active debug task touched critical files and no passed
/// candidate matches the current tree (the same bar `debug_report` applies
/// before it lets a self-modifying task claim `pass`).
pub(super) async fn check_guard(ctx: &DebugCtx, root: &Path) -> Result<(), String> {
    let Some(task) = ctx.store.active_task()? else {
        return Ok(());
    };
    let mut files = task.files_changed.clone();
    if let Some(f) = turn::diff_files(
        ctx,
        &root.display().to_string(),
        task.checkpoint_id.as_deref(),
    )
    .await
    {
        files.extend(f);
    }
    let assessment = selfmod::assess(&files);
    if !assessment.critical || candidate::candidate_valid_for_current_tree(ctx, root).await {
        return Ok(());
    }
    let shown: Vec<&str> = assessment
        .critical_files
        .iter()
        .take(5)
        .map(String::as_str)
        .collect();
    log::info!(
        "[debug_mode] local install refused task={} critical_files={}",
        task.id,
        assessment.critical_files.len()
    );
    Err(format!(
        "refused: the current debug task changed critical files ({}) and no validated \
         candidate matches the current tree. Run a candidate build first.",
        shown.join(", ")
    ))
}

fn read_version(root: &Path) -> Option<String> {
    let bytes = fs::read(root.join("app/package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}

fn finish_build(store: &InstallStore, outcome: Result<PathBuf, String>) {
    let r = store.modify(|rec| {
        if rec.phase != LocalInstallPhase::Building {
            return;
        }
        rec.finished_at = Some(chrono::Utc::now().to_rfc3339());
        match outcome {
            Ok(p) => {
                rec.phase = LocalInstallPhase::Ready;
                rec.bundle_path = Some(p.display().to_string());
                rec.error.clear();
            }
            Err(e) => {
                rec.phase = LocalInstallPhase::Failed;
                rec.error = tail(&e, ERROR_CAP);
            }
        }
    });
    match r {
        Ok(rec) => log::info!("[debug_mode] local install build phase={:?}", rec.phase),
        Err(e) => log::warn!("[debug_mode] local install: cannot persist build result: {e}"),
    }
}

/// Starts a build with injected steps. Refuses while one runs or while an
/// install is in flight.
pub(super) fn start_with(
    ctx: &DebugCtx,
    version: Option<String>,
    steps: Arc<dyn InstallSteps>,
) -> Result<LocalInstallRecord, String> {
    let dir = ctx.store.dir().to_path_buf();
    let store = InstallStore { dir: dir.clone() };
    let mut slots = lock(&RUNNING);
    slots.retain(|s| !s.handle.is_finished());
    if slots.iter().any(|s| s.dir == dir) {
        return Err("a local build is already running; wait for it to finish".into());
    }
    store.reconcile(std::process::id())?;
    if store.read()?.phase == LocalInstallPhase::Installing {
        return Err("an install is in progress; wait for the app to restart".into());
    }
    let rec = store.modify(|r| {
        *r = LocalInstallRecord {
            phase: LocalInstallPhase::Building,
            version,
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        }
    })?;
    log::info!(
        "[debug_mode] local install build start version={:?}",
        rec.version
    );
    let log_path = dir.join(LOG_FILE);
    let _ = fs::remove_file(&log_path);
    let handle = tokio::spawn(async move {
        let outcome = steps.build(&log_path).await;
        finish_build(&store, outcome);
    });
    slots.push(Slot { dir, handle });
    Ok(rec)
}

fn status_in(dir: &Path, my_pid: u32) -> Result<LocalInstallRecord, String> {
    let store = InstallStore {
        dir: dir.to_path_buf(),
    };
    store.reconcile(my_pid)?;
    let mut rec = store.read()?;
    if rec.phase != LocalInstallPhase::Idle {
        rec.log_tail = log_tail(&dir.join(LOG_FILE), LOG_TAIL_BYTES);
    }
    Ok(rec)
}

/// Starts the `apply` stage with an injected spawner: phase must be `ready`.
pub(super) fn apply_with(
    ctx: &DebugCtx,
    confirm: bool,
    root: &Path,
    app_pid: u32,
    spawner: &dyn HelperSpawner,
) -> Result<LocalInstallRecord, String> {
    if !confirm {
        return Err("confirm must be true to install and restart".into());
    }
    let dir = ctx.store.dir().to_path_buf();
    let store = InstallStore { dir: dir.clone() };
    store.reconcile(app_pid)?;
    let rec = store.read()?;
    if rec.phase != LocalInstallPhase::Ready {
        return Err(
            format!("nothing to install: phase is {:?}, not ready", rec.phase).to_lowercase(),
        );
    }
    let bundle = PathBuf::from(rec.bundle_path.clone().ok_or("no bundle recorded")?);
    if !bundle.join("Contents/Info.plist").is_file() {
        return Err(format!("the built bundle is gone: {}", bundle.display()));
    }
    let script = root.join(HELPER_SCRIPT);
    if !script.is_file() {
        return Err(format!("installer helper missing: {}", script.display()));
    }
    store.modify(|r| {
        r.phase = LocalInstallPhase::Installing;
        r.app_pid = Some(app_pid);
        r.installing_since = Some(chrono::Utc::now().to_rfc3339());
    })?;
    if let Err(e) = spawner.spawn(&script, &bundle, app_pid) {
        log::warn!("[debug_mode] local install helper failed to start: {e}");
        let _ = store.modify(|r| {
            r.phase = LocalInstallPhase::Ready;
            r.app_pid = None;
            r.installing_since = None;
        });
        return Err(e);
    }
    log::info!(
        "[debug_mode] local install applying bundle={}",
        bundle.display()
    );
    store.read()
}

fn data_dir() -> PathBuf {
    if let Ok(w) = crate::neppy::util::env::var("NEPPY_WORKSPACE") {
        if !w.is_empty() {
            return PathBuf::from(w);
        }
    }
    crate::neppy::config::default_root_neppy_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("neppy"))
}

/// Reads the installer's verdict from `state_dir`; `acknowledge` marks it seen.
pub(super) fn result_in(
    state_dir: &Path,
    acknowledge: bool,
) -> Result<Option<LocalInstallResult>, String> {
    let bytes = match fs::read(state_dir.join(RESULT_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {RESULT_FILE}: {e}")),
    };
    let mut res: LocalInstallResult = match serde_json::from_slice(&bytes) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[debug_mode] {RESULT_FILE} unreadable ({e}); ignoring");
            return Ok(None);
        }
    };
    let seen_path = state_dir.join(SEEN_FILE);
    if acknowledge {
        fs::write(&seen_path, &res.ts)
            .map_err(|e| format!("cannot record acknowledgement: {e}"))?;
    }
    res.seen = fs::read_to_string(&seen_path).is_ok_and(|s| s.trim() == res.ts);
    Ok(Some(res))
}

/// `debug_mode_install_local_build`.
pub async fn start(ctx: &DebugCtx, project_root: Option<&str>) -> RpcResult<LocalInstallRecord> {
    let r = async {
        let root = ctx.resolve_root(project_root).await?;
        check_guard(ctx, &root).await?;
        let steps = Arc::new(TauriSteps {
            target_dir: ctx.store.dir().join(TARGET_DIR),
            root: root.clone(),
            timeout: BUILD_TIMEOUT,
        });
        start_with(ctx, read_version(&root), steps)
    }
    .await;
    ops::audit(ctx, "install_local_build", "-", &r);
    r.and_then(done)
}

/// `debug_mode_install_local_status`.
pub async fn status(ctx: &DebugCtx) -> RpcResult<LocalInstallRecord> {
    done(status_in(ctx.store.dir(), std::process::id())?)
}

/// `debug_mode_install_local_apply`.
pub async fn apply(
    ctx: &DebugCtx,
    confirm: bool,
    project_root: Option<&str>,
) -> RpcResult<LocalInstallRecord> {
    let r = async {
        let root = ctx.resolve_root(project_root).await?;
        if confirm {
            check_guard(ctx, &root).await?;
        }
        apply_with(ctx, confirm, &root, std::process::id(), &ShellSpawner)
    }
    .await;
    ops::audit(ctx, "install_local_apply", "-", &r);
    r.and_then(done)
}

/// `debug_mode_install_local_result`.
pub async fn result(acknowledge: bool) -> RpcResult<Option<LocalInstallResult>> {
    done(result_in(&data_dir().join("debug_mode"), acknowledge)?)
}

#[cfg(test)]
#[path = "local_install_tests.rs"]
mod tests;
