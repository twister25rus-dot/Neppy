//! Wire and storage types for the local assistant, plus the bounds every
//! stored value is held to.
//!
//! Everything a task persists has an explicit cap here, so the state database
//! cannot grow with the length of a task or with what a small model chooses to
//! emit. Text is clipped on write, never rejected, except edit payloads: a
//! truncated `search`/`replace` would be a different edit.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub type TaskId = String;
pub type Result<T> = std::result::Result<T, AssistantError>;

/// Cumulative task summary, bytes.
pub const SUMMARY_MAX: usize = 2048;
/// Decisions kept per task, and the length of each.
pub const DECISIONS_MAX: usize = 20;
pub const DECISION_MAX_CHARS: usize = 300;
/// The next-step note, bytes.
pub const NEXT_STEP_MAX: usize = 1024;
/// Serialized plan of one step, bytes.
pub const PLAN_JSON_MAX: usize = 16 * 1024;
/// A completed step's plan is truncated to this once the task finishes.
pub const PLAN_JSON_DONE_MAX: usize = 2048;
/// Test output kept on the task record and fed back into a prompt, bytes.
pub const TEST_TAIL_STORE: usize = 2048;
/// Test output captured while the command runs, bytes.
pub const TEST_TAIL_RUN: usize = 4096;
/// Changed files remembered per task.
pub const CHANGED_FILES_MAX: usize = 200;
/// Result payload of one effect, bytes.
pub const EFFECT_RESULT_MAX: usize = 4096;
/// Edits one step may carry, and each edit field's size, bytes.
pub const EDITS_PER_STEP: usize = 8;
pub const EDIT_FIELD_MAX: usize = 8 * 1024;
/// Model-proposed search queries per step.
pub const QUERIES_PER_STEP: usize = 6;
/// Longest error text kept on a failed task, bytes.
pub const ERROR_MAX: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum AssistantError {
    #[error("the local assistant queue is full ({0} tasks pending)")]
    QueueFull(usize),
    #[error("task not found: {0}")]
    NotFound(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("the local assistant is disabled")]
    Disabled,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("model error: {0}")]
    Model(String),
    /// The step stopped without failing: preempted, paused, or cancelled. Its
    /// durable state is intact and the task can be resumed.
    #[error("step interrupted: {0}")]
    Interrupted(String),
}

impl From<rusqlite::Error> for AssistantError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Storage(err.to_string())
    }
}

impl From<std::io::Error> for AssistantError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<serde_json::Error> for AssistantError {
    fn from(err: serde_json::Error) -> Self {
        Self::Storage(format!("json: {err}"))
    }
}

/// Clip `text` to at most `max` bytes on a char boundary.
pub fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Keep the last `max` bytes of `text` on a char boundary.
pub fn clip_tail(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// What `start_task` accepts.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    pub project_root: PathBuf,
    pub goal: String,
    /// Whether the model's edits are applied. Off by default: a task that only
    /// reads and reports needs no write access.
    #[serde(default)]
    pub allow_edits: bool,
    /// The only command the task may run, supplied by the user.
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub max_steps: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Paused,
    Interrupted,
    Done,
    Failed,
    BudgetExhausted,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Interrupted => "interrupted",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "interrupted" => Self::Interrupted,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "budget_exhausted" => Self::BudgetExhausted,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// No further steps will run without an explicit resume.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::BudgetExhausted | Self::Cancelled
        )
    }

    /// States `resume` may restart.
    pub fn is_resumable(self) -> bool {
        matches!(self, Self::Paused | Self::Interrupted | Self::Failed)
    }
}

/// Where a step is. Each state is written only after the work it names is
/// durable, so resume knows exactly what has and has not happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Started,
    Planned,
    Applying,
    Applied,
    Tested,
    Completed,
}

impl StepState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Planned => "planned",
            Self::Applying => "applying",
            Self::Applied => "applied",
            Self::Tested => "tested",
            Self::Completed => "completed",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "started" => Self::Started,
            "planned" => Self::Planned,
            "applying" => Self::Applying,
            "applied" => Self::Applied,
            "tested" => Self::Tested,
            "completed" => Self::Completed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditOp {
    /// Path relative to the project root.
    pub path: String,
    /// Text to replace. Must occur exactly once. Empty creates a new file.
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub replace: String,
}

/// What the model returns for one step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StepPlan {
    /// The updated cumulative summary of the whole task so far.
    pub summary: String,
    /// Decisions made in this step. A bare string is read as one decision.
    #[serde(deserialize_with = "string_or_list")]
    pub decisions: Vec<String>,
    pub edits: Vec<EditOp>,
    pub run_tests: bool,
    /// What the next step should do.
    pub next_step: String,
    /// The goal is met.
    pub done: bool,
    /// What to look up in the project for the next step. A bare string is
    /// read as one query.
    #[serde(deserialize_with = "string_or_list")]
    pub search_queries: Vec<String>,
}

/// Small models sometimes answer a list field with one string, or null. Take
/// both; anything else is still an error.
fn string_or_list<'de, D>(de: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<String>),
        One(String),
        Nothing(()),
    }
    Ok(match OneOrMany::deserialize(de)? {
        OneOrMany::Many(v) => v,
        OneOrMany::One(s) if s.trim().is_empty() => Vec::new(),
        OneOrMany::One(s) => vec![s],
        OneOrMany::Nothing(()) => Vec::new(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestResult {
    pub exit_code: i32,
    pub duration_ms: u64,
    /// Last bytes of combined stdout and stderr.
    pub tail: String,
    #[serde(default)]
    pub timed_out: bool,
}

impl TestResult {
    pub fn passed(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }
}

/// Exit code recorded for a test command the policy refused to run.
pub const TEST_REFUSED: i32 = -2;

#[derive(Debug, Clone, Serialize)]
pub struct TaskRecord {
    pub id: TaskId,
    pub project_root: String,
    pub goal: String,
    pub status: TaskStatus,
    pub allow_edits: bool,
    pub test_command: Option<String>,
    pub max_steps: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub summary: String,
    pub decisions: Vec<String>,
    pub changed_files: Vec<String>,
    pub last_test: Option<TestResult>,
    pub next_step: String,
    pub completion_tokens_used: u64,
    pub steps_done: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StepRecord {
    pub task_id: TaskId,
    pub step_no: u32,
    pub state: StepState,
    pub plan: Option<StepPlan>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Everything one completed step changes on the task record, applied in one
/// transaction together with the step's own state.
#[derive(Debug, Clone, Default)]
pub struct TaskUpdate {
    pub summary: String,
    pub new_decisions: Vec<String>,
    pub new_changed_files: Vec<String>,
    pub last_test: Option<TestResult>,
    pub next_step: String,
    /// Set when this step ends the task.
    pub finish: Option<TaskStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Edit,
    Test,
}

impl EffectKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    /// Recorded, not yet applied.
    Intent,
    Applied,
    /// The file was not in the state the edit expected; nothing was written.
    Conflict,
    /// Policy refused it; nothing was written.
    Refused,
    /// Its step completed.
    Done,
}

impl EffectStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Applied => "applied",
            Self::Conflict => "conflict",
            Self::Refused => "refused",
            Self::Done => "done",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "intent" => Self::Intent,
            "applied" => Self::Applied,
            "conflict" => Self::Conflict,
            "refused" => Self::Refused,
            "done" => Self::Done,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EffectRecord {
    pub effect_key: String,
    pub task_id: TaskId,
    pub step_no: u32,
    pub kind: EffectKind,
    pub path: String,
    pub pre_sha: String,
    pub post_sha: String,
    pub status: EffectStatus,
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    Applied,
    AlreadyApplied,
    Conflict(String),
    Refused(String),
}

/// A retrieved piece of the project. Never more than one chunk or window, and
/// read back from disk at retrieval time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    pub path: String,
    pub start: u32,
    pub end: u32,
    pub text: String,
    pub why: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip("héllo", 2), "h");
        assert_eq!(clip_tail("héllo", 3), "llo");
        assert_eq!(clip("abc", 10), "abc");
    }

    #[test]
    fn statuses_round_trip() {
        for status in [
            TaskStatus::Queued,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Interrupted,
            TaskStatus::Done,
            TaskStatus::Failed,
            TaskStatus::BudgetExhausted,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(TaskStatus::parse(status.as_str()), Some(status));
        }
        for state in [
            StepState::Started,
            StepState::Planned,
            StepState::Applying,
            StepState::Applied,
            StepState::Tested,
            StepState::Completed,
        ] {
            assert_eq!(StepState::parse(state.as_str()), Some(state));
        }
        assert!(StepState::Planned < StepState::Applying);
        assert!(TaskStatus::Done.is_terminal());
        assert!(TaskStatus::Interrupted.is_resumable());
        assert!(!TaskStatus::BudgetExhausted.is_resumable());
    }
}
