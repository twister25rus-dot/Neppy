//! Persistence for Debug Mode under `{workspace_dir}/debug_mode/`:
//! `history.json` (tasks), `checkpoints.json`, and `audit.jsonl`.
//!
//! History lives in the core workspace (not the project) because agent tools
//! cannot write there, so a misbehaving debug agent cannot rewrite its own
//! audit trail. JSON files are written atomically (temp file + rename).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::types::{AuditEntry, Checkpoint, TaskRecord};

/// Oldest tasks are dropped past this many records.
pub const MAX_TASKS: usize = 500;
/// `audit.jsonl` is rotated to `audit.jsonl.1` past this size.
pub const AUDIT_ROTATE_BYTES: u64 = 2 * 1024 * 1024;

const HISTORY: &str = "history.json";
const CHECKPOINTS: &str = "checkpoints.json";
const AUDIT: &str = "audit.jsonl";

/// Serialises read-modify-write cycles across concurrent RPC calls.
static STORE_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone)]
pub struct DebugStore {
    dir: PathBuf,
}

impl DebugStore {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            dir: workspace_dir.join("debug_mode"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read_json<T: DeserializeOwned + Default>(&self, name: &str) -> Result<T, String> {
        let path = self.dir.join(name);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
            Err(e) => return Err(format!("[debug_mode] cannot read {name}: {e}")),
        };
        match serde_json::from_slice(&bytes) {
            Ok(v) => Ok(v),
            Err(e) => {
                // Keep the unreadable file for forensics rather than losing it.
                let aside = self
                    .dir
                    .join(format!("{name}.corrupt-{}", chrono::Utc::now().timestamp()));
                log::warn!("[debug_mode] {name} unreadable ({e}); moving aside and starting fresh");
                let _ = fs::rename(&path, &aside);
                Ok(T::default())
            }
        }
    }

    fn write_json<T: Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("[debug_mode] create dir: {e}"))?;
        let tmp = self
            .dir
            .join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let data =
            serde_json::to_vec_pretty(value).map_err(|e| format!("serialize {name}: {e}"))?;
        let write = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            fs::rename(&tmp, self.dir.join(name))
        };
        write().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("[debug_mode] write {name}: {e}")
        })
    }

    // ── checkpoints ──────────────────────────────────────────────────────

    pub fn checkpoint_add(&self, cp: Checkpoint) -> Result<(), String> {
        let _g = lock();
        let mut all: Vec<Checkpoint> = self.read_json(CHECKPOINTS)?;
        all.push(cp);
        self.write_json(CHECKPOINTS, &all)
    }

    /// Newest first.
    pub fn checkpoint_list(&self, limit: usize) -> Result<Vec<Checkpoint>, String> {
        let _g = lock();
        let all: Vec<Checkpoint> = self.read_json(CHECKPOINTS)?;
        Ok(all.into_iter().rev().take(limit).collect())
    }

    pub fn checkpoint_get(&self, id: &str) -> Result<Option<Checkpoint>, String> {
        let _g = lock();
        let all: Vec<Checkpoint> = self.read_json(CHECKPOINTS)?;
        Ok(all.into_iter().find(|c| c.id == id))
    }

    // ── tasks ────────────────────────────────────────────────────────────

    pub fn task_add(&self, task: TaskRecord) -> Result<(), String> {
        let _g = lock();
        let mut all: Vec<TaskRecord> = self.read_json(HISTORY)?;
        all.push(task);
        if all.len() > MAX_TASKS {
            let cut = all.len() - MAX_TASKS;
            all.drain(..cut);
        }
        self.write_json(HISTORY, &all)
    }

    /// Apply `f` to the task and persist it. `Ok(None)` when the id is unknown.
    pub fn task_modify<F: FnOnce(&mut TaskRecord)>(
        &self,
        id: &str,
        f: F,
    ) -> Result<Option<TaskRecord>, String> {
        let _g = lock();
        let mut all: Vec<TaskRecord> = self.read_json(HISTORY)?;
        let Some(task) = all.iter_mut().find(|t| t.id == id) else {
            return Ok(None);
        };
        f(task);
        task.updated_at = chrono::Utc::now().to_rfc3339();
        let updated = task.clone();
        self.write_json(HISTORY, &all)?;
        Ok(Some(updated))
    }

    pub fn task_get(&self, id: &str) -> Result<Option<TaskRecord>, String> {
        let _g = lock();
        let all: Vec<TaskRecord> = self.read_json(HISTORY)?;
        Ok(all.into_iter().find(|t| t.id == id))
    }

    /// Newest first.
    pub fn task_list(&self, limit: usize) -> Result<Vec<TaskRecord>, String> {
        let _g = lock();
        let all: Vec<TaskRecord> = self.read_json(HISTORY)?;
        Ok(all.into_iter().rev().take(limit).collect())
    }

    /// The active task: the NEWEST task, and only when it is still planning /
    /// editing / validating. An older task stranded in an active status never
    /// counts once a newer task exists (it would otherwise resurface as "the
    /// active task" after the newer ones finish); see [`Self::abandon_active_except`].
    pub fn active_task(&self) -> Result<Option<TaskRecord>, String> {
        let _g = lock();
        let all: Vec<TaskRecord> = self.read_json(HISTORY)?;
        let newest = all.into_iter().next_back();
        if let Some(t) = &newest {
            log::trace!(
                "[debug_mode] active_task newest={} status={:?} active={}",
                t.id,
                t.status,
                t.status.is_active()
            );
        }
        Ok(newest.filter(|t| t.status.is_active()))
    }

    /// Closes every task other than `keep_id` that is still in an active
    /// status: `Failed`, with `abandoned: superseded by <keep_id>` recorded in
    /// the summary. Returns the ids that were closed (oldest first).
    pub fn abandon_active_except(&self, keep_id: &str) -> Result<Vec<String>, String> {
        let _g = lock();
        let mut all: Vec<TaskRecord> = self.read_json(HISTORY)?;
        let reason = format!("abandoned: superseded by {keep_id}");
        let now = chrono::Utc::now().to_rfc3339();
        let mut closed = Vec::new();
        for t in all
            .iter_mut()
            .filter(|t| t.id != keep_id && t.status.is_active())
        {
            log::info!(
                "[debug_mode] abandoning stale task={} status={:?} superseded_by={keep_id}",
                t.id,
                t.status
            );
            t.status = super::types::TaskStatus::Failed;
            t.summary = Some(match t.summary.take() {
                Some(prev) if !prev.trim().is_empty() => format!("{prev}\n{reason}"),
                _ => reason.clone(),
            });
            t.updated_at = now.clone();
            closed.push(t.id.clone());
        }
        if !closed.is_empty() {
            self.write_json(HISTORY, &all)?;
        }
        Ok(closed)
    }

    // ── audit ────────────────────────────────────────────────────────────

    pub fn audit_append(&self, entry: &AuditEntry) -> Result<(), String> {
        let _g = lock();
        fs::create_dir_all(&self.dir).map_err(|e| format!("[debug_mode] create dir: {e}"))?;
        let path = self.dir.join(AUDIT);
        if fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > AUDIT_ROTATE_BYTES {
            let _ = fs::rename(&path, self.dir.join(format!("{AUDIT}.1")));
        }
        let mut line = serde_json::to_string(entry).map_err(|e| e.to_string())?;
        line.push('\n');
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("[debug_mode] open audit: {e}"))?;
        f.write_all(line.as_bytes())
            .map_err(|e| format!("[debug_mode] write audit: {e}"))
    }

    /// Last `limit` entries, oldest first.
    pub fn audit_tail(&self, limit: usize) -> Result<Vec<AuditEntry>, String> {
        let _g = lock();
        let text = match fs::read_to_string(self.dir.join(AUDIT)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(format!("[debug_mode] read audit: {e}")),
        };
        let mut entries: Vec<AuditEntry> = text
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if entries.len() > limit {
            entries.drain(..entries.len() - limit);
        }
        Ok(entries)
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
