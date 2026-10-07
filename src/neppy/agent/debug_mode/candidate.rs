//! Staged self-update candidates (spec sections 35-36).
//!
//! The running app is a built binary, so editing source does not change it.
//! A *candidate* proves the modified source is sound: build it in an isolated
//! target dir, launch it as a separate process, health-check it, and only then
//! mark it `passed`. The running app and its `target/` are never touched; the
//! previously working binary is copied to `known_good/` (newest 3 kept) before
//! a candidate may pass.
//!
//! Records live in `{workspace}/debug_mode/candidates.json` (atomic writes).
//! At most one candidate runs at a time. The real build / launch steps are in
//! [`super::candidate_steps`]; the state machine here takes them as a trait so
//! tests never run cargo.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;

use super::candidate_steps::{binary_path, tail, CargoSteps, Steps, ERROR_TAIL_CAP};
use super::git::{resolve_project_root, Git};
use super::ops::{self, done, DebugCtx, RpcResult};
use super::types::{CandidatePhase, CandidateRecord};

const FILE: &str = "candidates.json";
const MAX_CANDIDATES: usize = 50;
/// How many saved known-good binaries are kept.
pub(super) const KEEP_KNOWN_GOOD: usize = 3;
pub(super) const KNOWN_GOOD_DIR: &str = "known_good";
const KNOWN_GOOD_PREFIX: &str = "neppy-core-";

static FILE_LOCK: Mutex<()> = Mutex::new(());

struct Slot {
    dir: PathBuf,
    handle: JoinHandle<()>,
}

/// Live background pipelines, keyed by store dir (one per workspace).
static RUNNING: Mutex<Vec<Slot>> = Mutex::new(Vec::new());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn is_live(dir: &Path) -> bool {
    let mut slots = lock(&RUNNING);
    slots.retain(|s| !s.handle.is_finished());
    slots.iter().any(|s| s.dir == dir)
}

/// File-backed candidate records.
#[derive(Debug, Clone)]
pub(super) struct CandidateStore {
    dir: PathBuf,
}

impl CandidateStore {
    pub(super) fn new(ctx: &DebugCtx) -> Self {
        Self {
            dir: ctx.store.dir().to_path_buf(),
        }
    }

    fn read<T: DeserializeOwned + Default>(&self) -> Result<T, String> {
        let path = self.dir.join(FILE);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
            Err(e) => return Err(format!("[debug_mode] cannot read {FILE}: {e}")),
        };
        serde_json::from_slice(&bytes).or_else(|e| {
            log::warn!("[debug_mode] {FILE} unreadable ({e}); moving aside");
            let aside = self
                .dir
                .join(format!("{FILE}.corrupt-{}", chrono::Utc::now().timestamp()));
            let _ = fs::rename(&path, aside);
            Ok(T::default())
        })
    }

    fn write(&self, all: &Vec<CandidateRecord>) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("[debug_mode] create dir: {e}"))?;
        let tmp = self
            .dir
            .join(format!(".{FILE}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let data = serde_json::to_vec_pretty(all).map_err(|e| e.to_string())?;
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

    pub(super) fn add(&self, rec: CandidateRecord) -> Result<(), String> {
        let _g = lock(&FILE_LOCK);
        let mut all: Vec<CandidateRecord> = self.read()?;
        all.push(rec);
        if all.len() > MAX_CANDIDATES {
            let cut = all.len() - MAX_CANDIDATES;
            all.drain(..cut);
        }
        self.write(&all)
    }

    pub(super) fn modify<F: FnOnce(&mut CandidateRecord)>(
        &self,
        id: &str,
        f: F,
    ) -> Result<Option<CandidateRecord>, String> {
        let _g = lock(&FILE_LOCK);
        let mut all: Vec<CandidateRecord> = self.read()?;
        let Some(rec) = all.iter_mut().find(|r| r.id == id) else {
            return Ok(None);
        };
        f(rec);
        let out = rec.clone();
        self.write(&all)?;
        Ok(Some(out))
    }

    pub(super) fn get(&self, id: &str) -> Result<Option<CandidateRecord>, String> {
        let _g = lock(&FILE_LOCK);
        Ok(self
            .read::<Vec<CandidateRecord>>()?
            .into_iter()
            .find(|r| r.id == id))
    }

    /// Oldest first.
    pub(super) fn all(&self) -> Result<Vec<CandidateRecord>, String> {
        let _g = lock(&FILE_LOCK);
        self.read()
    }

    pub(super) fn latest(&self) -> Result<Option<CandidateRecord>, String> {
        Ok(self.all()?.pop())
    }

    /// A record still marked running with no live pipeline (the app restarted
    /// mid-build, or the task died) can never finish: mark it failed.
    fn reconcile(&self) {
        if !is_live(&self.dir) {
            self.fail_stale();
        }
    }

    /// [`reconcile`](Self::reconcile) for a caller that already holds
    /// `RUNNING` and knows no pipeline is live for this dir.
    fn fail_stale(&self) {
        let Ok(all) = self.all() else { return };
        for rec in all.iter().filter(|r| r.phase.is_running()) {
            log::warn!("[debug_mode] candidate {} was interrupted", rec.id);
            let _ = self.modify(&rec.id, |r| {
                r.phase = CandidatePhase::Failed;
                r.finished_at = Some(chrono::Utc::now().to_rfc3339());
                r.error_tail = "interrupted: the candidate pipeline is no longer running".into();
            });
        }
    }
}

/// Copies `src` into `dest_dir/neppy-core-<timestamp>` (atomically) and prunes
/// to the newest [`KEEP_KNOWN_GOOD`]. `Ok(None)` when `src` does not exist.
pub(super) fn preserve_known_good(src: &Path, dest_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !src.is_file() {
        log::debug!(
            "[debug_mode] no current binary at {}; nothing to preserve",
            src.display()
        );
        return Ok(None);
    }
    fs::create_dir_all(dest_dir).map_err(|e| format!("create {}: {e}", dest_dir.display()))?;
    let name = format!(
        "{KNOWN_GOOD_PREFIX}{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f")
    );
    let (tmp, dest) = (dest_dir.join(format!(".{name}.tmp")), dest_dir.join(&name));
    fs::copy(src, &tmp)
        .and_then(|_| fs::rename(&tmp, &dest))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("copy {} -> {}: {e}", src.display(), dest.display())
        })?;
    let mut saved: Vec<PathBuf> = fs::read_dir(dest_dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(KNOWN_GOOD_PREFIX))
        })
        .collect();
    saved.sort();
    while saved.len() > KEEP_KNOWN_GOOD {
        let old = saved.remove(0);
        log::debug!("[debug_mode] pruning old known-good {}", old.display());
        let _ = fs::remove_file(old);
    }
    Ok(Some(dest))
}

fn finish(store: &CandidateStore, id: &str, f: impl FnOnce(&mut CandidateRecord)) {
    let r = store.modify(id, |r| {
        // `cancel` is final; a late pipeline write must not resurrect it.
        if r.phase == CandidatePhase::Cancelled {
            return;
        }
        f(r);
        r.finished_at = Some(chrono::Utc::now().to_rfc3339());
    });
    if let Err(e) = r {
        log::warn!("[debug_mode] candidate {id}: cannot persist result: {e}");
    }
}

fn set_phase(store: &CandidateStore, id: &str, phase: CandidatePhase) {
    log::info!("[debug_mode] candidate {id} phase={phase:?}");
    let r = store.modify(id, |r| {
        if r.phase.is_running() {
            r.phase = phase;
        }
    });
    if let Err(e) = r {
        log::warn!("[debug_mode] candidate {id}: cannot persist phase: {e}");
    }
}

fn fail(store: &CandidateStore, id: &str, error: &str, build_ok: bool) {
    let err = tail(error, ERROR_TAIL_CAP);
    log::warn!("[debug_mode] candidate {id} failed build_ok={build_ok}");
    finish(store, id, |r| {
        r.phase = CandidatePhase::Failed;
        r.build_ok = build_ok;
        r.error_tail = err;
    });
}

/// build -> launch -> health -> preserve known-good -> passed.
pub(super) async fn run_pipeline(
    store: CandidateStore,
    id: String,
    steps: Arc<dyn Steps>,
    known_good_src: PathBuf,
    known_good_dir: PathBuf,
) {
    if let Err(e) = steps.build().await {
        return fail(&store, &id, &e, false);
    }
    let _ = store.modify(&id, |r| r.build_ok = true);
    set_phase(&store, &id, CandidatePhase::Launching);
    let mut launched = match steps.launch().await {
        Ok(l) => l,
        Err(e) => return fail(&store, &id, &e, true),
    };
    set_phase(&store, &id, CandidatePhase::HealthCheck);
    let health = steps.health(&mut launched).await;
    steps.stop(launched).await;
    if let Err(e) = health {
        return fail(&store, &id, &e, true);
    }
    let saved = match preserve_known_good(&known_good_src, &known_good_dir) {
        Ok(p) => p,
        Err(e) => {
            return fail(
                &store,
                &id,
                &format!("could not preserve the current binary as known-good: {e}"),
                true,
            )
        }
    };
    log::info!("[debug_mode] candidate {id} passed known_good={saved:?}");
    finish(&store, &id, |r| {
        r.phase = CandidatePhase::Passed;
        r.build_ok = true;
        r.health_ok = true;
        r.known_good_path = saved.map(|p| p.display().to_string());
    });
}

/// Starts a candidate pipeline with injected steps. Refuses while one runs.
pub(super) fn start_with(
    ctx: &DebugCtx,
    tree_hash: String,
    task_id: Option<String>,
    steps: Arc<dyn Steps>,
    known_good_src: PathBuf,
) -> Result<CandidateRecord, String> {
    if let Some(t) = task_id.as_deref() {
        if ctx.store.task_get(t)?.is_none() {
            return Err(format!("unknown task '{t}'"));
        }
    }
    let store = CandidateStore::new(ctx);
    let dir = ctx.store.dir().to_path_buf();
    let mut slots = lock(&RUNNING);
    slots.retain(|s| !s.handle.is_finished());
    if slots.iter().any(|s| s.dir == dir) {
        return Err("a candidate is already running; wait for it or cancel it".into());
    }
    store.fail_stale();
    let now = chrono::Utc::now();
    let rec = CandidateRecord {
        id: format!(
            "cand-{}-{}",
            now.format("%Y%m%d%H%M%S"),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ),
        task_id: task_id.clone(),
        tree_hash,
        started_at: now.to_rfc3339(),
        finished_at: None,
        phase: CandidatePhase::Building,
        build_ok: false,
        health_ok: false,
        error_tail: String::new(),
        known_good_path: None,
    };
    store.add(rec.clone())?;
    if let Some(t) = task_id.as_deref() {
        let id = rec.id.clone();
        let _ = ctx
            .store
            .task_modify(t, move |task| task.candidate_id = Some(id));
    }
    log::info!(
        "[debug_mode] candidate {} start task={} tree={}",
        rec.id,
        task_id.as_deref().unwrap_or("-"),
        rec.tree_hash
    );
    let kg_dir = dir.join(KNOWN_GOOD_DIR);
    let handle = tokio::spawn(run_pipeline(
        store,
        rec.id.clone(),
        steps,
        known_good_src,
        kg_dir,
    ));
    slots.push(Slot { dir, handle });
    Ok(rec)
}

/// `debug_mode_candidate_start`: snapshot the tree, then build + health-check
/// in the background.
pub async fn start(
    ctx: &DebugCtx,
    project_root: Option<&str>,
    task_id: Option<&str>,
) -> RpcResult<CandidateRecord> {
    let r = async {
        let root = resolve_project_root(project_root).await?;
        let tree = Git::new(&root).snapshot_trees().await?.worktree_tree;
        let steps = Arc::new(CargoSteps {
            root: root.clone(),
            target_dir: ctx.store.dir().join("candidate-target"),
        });
        // The binary the user is running today comes from the repo's own target.
        let current = binary_path(&root.join("target"));
        start_with(ctx, tree, task_id.map(str::to_string), steps, current)
    }
    .await;
    ops::audit(ctx, "candidate_start", task_id.unwrap_or("-"), &r);
    r.and_then(done)
}

/// `debug_mode_candidate_status`: a record by id, else the latest (`None`
/// when there are none).
pub async fn status(
    ctx: &DebugCtx,
    candidate_id: Option<&str>,
) -> RpcResult<Option<CandidateRecord>> {
    let r = (|| {
        let store = CandidateStore::new(ctx);
        store.reconcile();
        match candidate_id {
            Some(id) => store
                .get(id)?
                .map(Some)
                .ok_or_else(|| format!("unknown candidate '{id}'")),
            None => store.latest(),
        }
    })();
    ops::audit(
        ctx,
        "candidate_status",
        candidate_id.unwrap_or("latest"),
        &r,
    );
    r.and_then(done)
}

/// `debug_mode_candidate_cancel`: abort the running pipeline (its process
/// groups die with the dropped futures) and record `cancelled`.
pub async fn cancel(ctx: &DebugCtx) -> RpcResult<CandidateRecord> {
    let r = (|| {
        let store = CandidateStore::new(ctx);
        let dir = ctx.store.dir().to_path_buf();
        let aborted = {
            let mut slots = lock(&RUNNING);
            let mut any = false;
            for s in slots.iter().filter(|s| s.dir == dir) {
                s.handle.abort();
                any = true;
            }
            slots.retain(|s| s.dir != dir);
            any
        };
        let running = store
            .all()?
            .into_iter()
            .rev()
            .find(|r| r.phase.is_running())
            .ok_or("no candidate is running")?;
        log::info!(
            "[debug_mode] candidate {} cancelled (aborted={aborted})",
            running.id
        );
        store
            .modify(&running.id, |r| {
                r.phase = CandidatePhase::Cancelled;
                r.finished_at = Some(chrono::Utc::now().to_rfc3339());
            })?
            .ok_or_else(|| "candidate vanished".to_string())
    })();
    ops::audit(ctx, "candidate_cancel", "-", &r);
    r.and_then(done)
}

/// Polls until the candidate stops running or `timeout` passes (the returned
/// record then still shows a running phase).
pub async fn wait(
    ctx: &DebugCtx,
    candidate_id: &str,
    timeout: Duration,
    poll: Duration,
) -> Result<CandidateRecord, String> {
    let store = CandidateStore::new(ctx);
    let deadline = Instant::now() + timeout;
    loop {
        store.reconcile();
        let rec = store
            .get(candidate_id)?
            .ok_or_else(|| format!("unknown candidate '{candidate_id}'"))?;
        if !rec.phase.is_running() || Instant::now() >= deadline {
            return Ok(rec);
        }
        tokio::time::sleep(poll).await;
    }
}

/// True when a passed candidate was built from exactly `tree_hash`.
pub(super) fn valid_for_tree(ctx: &DebugCtx, tree_hash: &str) -> bool {
    CandidateStore::new(ctx).all().is_ok_and(|all| {
        all.iter()
            .any(|r| r.phase == CandidatePhase::Passed && r.tree_hash == tree_hash)
    })
}

/// True when a passed candidate matches the current working tree of `root`
/// (read-only: the tree is hashed through a temporary index).
pub async fn candidate_valid_for_current_tree(ctx: &DebugCtx, root: &Path) -> bool {
    match Git::new(root).snapshot_trees().await {
        Ok(t) => valid_for_tree(ctx, &t.worktree_tree),
        Err(e) => {
            log::warn!("[debug_mode] cannot hash current tree: {e}");
            false
        }
    }
}

#[cfg(test)]
#[path = "candidate_tests.rs"]
mod tests;
