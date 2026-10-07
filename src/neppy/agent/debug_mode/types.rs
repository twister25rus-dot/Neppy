//! Serde types for Debug Mode (Phase 1: core domain, no UI / no agent).

use serde::{Deserialize, Serialize};

/// Max bytes of stdout / stderr kept from a validation command.
pub const OUTPUT_TAIL_CAP: usize = 32 * 1024;
/// Max bytes of unified diff text returned by `diff`.
pub const DIFF_CAP: usize = 200 * 1024;
/// Default `run_check` timeout.
pub const DEFAULT_CHECK_TIMEOUT_SECS: u64 = 600;
/// Upper bound a caller may request for `run_check`.
pub const MAX_CHECK_TIMEOUT_SECS: u64 = 3600;
/// Env var consulted when no explicit `project_root` is given.
pub const PROJECT_ROOT_ENV: &str = "NEPPY_DEBUG_PROJECT_ROOT";
/// Ref namespace under which checkpoint snapshots are pinned.
pub const CHECKPOINT_REF_PREFIX: &str = "refs/neppy-debug/checkpoints/";

/// Working-tree changes parsed from `git status --porcelain`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyFiles {
    pub modified: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub untracked: Vec<String>,
}

impl DirtyFiles {
    pub fn is_clean(&self) -> bool {
        self.modified.is_empty()
            && self.added.is_empty()
            && self.deleted.is_empty()
            && self.untracked.is_empty()
    }

    /// Every tracked-file change (everything except untracked), sorted.
    pub fn tracked_changes(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .modified
            .iter()
            .chain(&self.added)
            .chain(&self.deleted)
            .cloned()
            .collect();
        v.sort();
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugStatus {
    pub project_root: String,
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    /// `None` on an unborn branch (no commits yet).
    pub head: Option<String>,
    pub dirty: DirtyFiles,
    pub task_active: bool,
    pub active_task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Lint,
    Typecheck,
    Test,
    Build,
    Format,
}

/// One validation command discovered from the project's own files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugCheck {
    pub id: String,
    pub label: String,
    /// argv, never a shell string.
    pub command: Vec<String>,
    pub kind: CheckKind,
    /// True for the project's own `debug:check` aggregate script.
    #[serde(default)]
    pub preferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub description: String,
    pub task_id: Option<String>,
    pub project_root: String,
    pub created_at: String,
    pub head: String,
    pub branch: Option<String>,
    /// Commit pinned under `refs/neppy-debug/checkpoints/<id>`; its tree holds
    /// tracked and untracked (non-ignored) files. Equals `head` when the
    /// working tree matched HEAD exactly.
    pub snapshot_sha: String,
    /// Tree of the staged state, pinned under `<id>-index`. Empty on legacy
    /// records (rollback then leaves the index equal to the snapshot).
    #[serde(default)]
    pub index_tree: String,
    /// Tracked files that differed from HEAD (any state).
    pub dirty_files: Vec<String>,
    /// Untracked (non-ignored) files present at checkpoint time.
    pub untracked_files: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Planning,
    Editing,
    Validating,
    Pass,
    Partial,
    Failed,
    RolledBack,
}

impl TaskStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Planning | Self::Editing | Self::Validating)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRecord {
    pub check_id: Option<String>,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub at: String,
    /// Short tail of combined output (capped when stored in task history).
    #[serde(default)]
    pub output_tail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub request: String,
    pub created_at: String,
    pub updated_at: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub files_changed: Vec<String>,
    #[serde(default)]
    pub validation: Vec<ValidationRecord>,
    pub summary: Option<String>,
    pub checkpoint_id: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    /// Critical (self-modification) files this task touched, per
    /// [`super::selfmod::assess`]. Absent in older history files.
    #[serde(default)]
    pub critical_files: Vec<String>,
    /// Latest staged-update candidate built for this task.
    #[serde(default)]
    pub candidate_id: Option<String>,
}

/// Partial update applied by `task_update`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPatch {
    pub status: Option<TaskStatus>,
    pub files_changed: Option<Vec<String>>,
    pub validation: Option<Vec<ValidationRecord>>,
    pub summary: Option<String>,
    pub checkpoint_id: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub check_id: Option<String>,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub passed: bool,
    pub timed_out: bool,
    pub duration_ms: u64,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RollbackResult {
    pub checkpoint_id: String,
    /// The auto-created "pre-rollback" checkpoint (roll forward with this).
    pub pre_rollback_checkpoint_id: String,
    pub restored: Vec<String>,
    pub removed: Vec<String>,
    /// HEAD at rollback time differs from the checkpoint's HEAD; the branch
    /// itself is never moved.
    pub head_moved: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitResult {
    pub task_id: String,
    /// Full sha of the new commit.
    pub commit: String,
    pub branch: Option<String>,
    /// The task's files that were staged and committed.
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DiffSummary {
    pub modified: usize,
    pub created: usize,
    pub deleted: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffFile {
    pub path: String,
    /// `None` for binary files.
    pub added: Option<u64>,
    pub deleted: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiffResult {
    /// What the working tree was compared against ("HEAD" or the checkpoint id).
    pub base: String,
    pub text: String,
    pub truncated: bool,
    pub summary: DiffSummary,
    pub files: Vec<DiffFile>,
    /// Untracked files (counted in `summary.created`, absent from `text`).
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: String,
    pub op: String,
    pub target: String,
    pub outcome: String,
}

/// Result of classifying a set of changed files against the critical-file list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfModAssessment {
    /// True when at least one changed file is critical.
    pub critical: bool,
    pub critical_files: Vec<String>,
}

/// Where a staged-update candidate is in its build / launch / health cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidatePhase {
    Building,
    Launching,
    HealthCheck,
    Passed,
    Failed,
    Cancelled,
}

impl CandidatePhase {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Building | Self::Launching | Self::HealthCheck)
    }
}

/// One staged-update candidate: a build of the modified source, launched as a
/// separate process and health-checked before it may be marked valid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateRecord {
    pub id: String,
    pub task_id: Option<String>,
    /// Working-tree hash the candidate was built from.
    pub tree_hash: String,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub phase: CandidatePhase,
    #[serde(default)]
    pub build_ok: bool,
    #[serde(default)]
    pub health_ok: bool,
    /// Tail of the build output or the health-check failure.
    #[serde(default)]
    pub error_tail: String,
    /// Copy of the previously working binary saved before this candidate passed.
    #[serde(default)]
    pub known_good_path: Option<String>,
}
