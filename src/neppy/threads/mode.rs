//! Per-thread operating mode: `chat` or `orchestration`.
//!
//! A thread is always in exactly one mode. The mode decides which delegation
//! surface the thread's agent is handed and how it is told to use it (see
//! `web_chat::mode`), and nothing else: the thread, its history, its memory and
//! its goal are the same on both sides of a switch, so changing mode mid-thread
//! carries the whole conversation across.
//!
//! # Persistence
//!
//! The mode rides the thread's existing `labels` list as one reserved label,
//! `mode:orchestration`. `chat` is the absence of a mode label, which is what
//! makes the field additive: every thread that predates it reads as `chat` with
//! no migration, and a store that does not know about modes simply keeps an
//! opaque label. Labels are the only per-thread metadata the conversation store
//! lets this crate extend (the record type lives in `tinycortex`).
//!
//! The reserved label never reaches a client through `labels` —
//! [`strip_mode_labels`] removes it from the summary and the mode is reported as
//! its own `mode` field instead — and `threads_update_labels` re-attaches it, so
//! a client that rewrites the user-visible labels cannot silently flip a thread
//! back to chat.
//!
//! # The ambient turn mode
//!
//! [`with_turn_mode`] scopes the mode over one agent turn as a task-local, the
//! same way `thread_context::with_thread_id` scopes the thread id. Tools that
//! must behave differently per mode without being rebuilt (the `delegate_*`
//! family runs blocking in chat mode) read [`current_turn_mode`]. An unset
//! task-local means "no mode was declared" (cron, CLI, sub-agent turns), and
//! callers must keep their legacy behaviour in that case rather than assuming
//! chat.

use serde::{Deserialize, Serialize};

use crate::neppy::memory::ConversationThreadSummary;

/// Prefix shared by every reserved mode label.
pub const MODE_LABEL_PREFIX: &str = "mode:";
/// The persisted marker for an orchestration-mode thread.
pub const ORCHESTRATION_LABEL: &str = "mode:orchestration";

/// Operating mode of a conversation thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadMode {
    /// One visible assistant with full capabilities. Complex work is handled
    /// by that assistant itself; no multi-agent fleet is exposed.
    #[default]
    Chat,
    /// Supervisor: the assistant decomposes the task, assigns workers, monitors
    /// them and recovers from failures.
    Orchestration,
}

impl ThreadMode {
    /// Wire spelling used by RPC params/results and events.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Orchestration => "orchestration",
        }
    }

    /// Parses the wire spelling (case-insensitive, trimmed). `None` for
    /// anything that is not a known mode, so a typo is an error at the RPC
    /// boundary instead of a silent fall-back to chat.
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "chat" => Some(Self::Chat),
            "orchestration" => Some(Self::Orchestration),
            _ => None,
        }
    }

    /// Reads the mode out of a thread's labels. No mode label means `chat`.
    pub fn from_labels(labels: &[String]) -> Self {
        if labels.iter().any(|l| l == ORCHESTRATION_LABEL) {
            Self::Orchestration
        } else {
            Self::Chat
        }
    }
}

impl std::fmt::Display for ThreadMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `labels` without any reserved mode label — what a client is shown.
pub fn strip_mode_labels(labels: Vec<String>) -> Vec<String> {
    labels
        .into_iter()
        .filter(|l| !l.starts_with(MODE_LABEL_PREFIX))
        .collect()
}

/// `labels` with exactly the label for `mode` (none for `chat`), preserving the
/// order of every other label.
pub fn labels_with_mode(labels: Vec<String>, mode: ThreadMode) -> Vec<String> {
    let mut out = strip_mode_labels(labels);
    if mode == ThreadMode::Orchestration {
        out.push(ORCHESTRATION_LABEL.to_string());
    }
    out
}

/// Params of `openhuman.threads_set_mode`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetThreadModeRequest {
    pub thread_id: String,
    /// `chat` or `orchestration`.
    pub mode: String,
    /// Free-form origin tag for the audit trail (`rpc` when omitted). Never
    /// message content.
    #[serde(default)]
    pub source: Option<String>,
}

/// Result of `openhuman.threads_set_mode`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadModeResult {
    /// The thread after the change (its `mode` field is the new mode).
    pub thread: ConversationThreadSummary,
    /// Mode the thread was in before this call.
    pub previous_mode: String,
    /// `false` when the thread was already in the requested mode (no event is
    /// published and nothing is written).
    pub changed: bool,
}

tokio::task_local! {
    static TURN_MODE: ThreadMode;
}

/// Runs `fut` with `mode` as the ambient turn mode.
pub async fn with_turn_mode<F>(mode: ThreadMode, fut: F) -> F::Output
where
    F: std::future::Future,
{
    TURN_MODE.scope(mode, fut).await
}

/// The mode declared for the current turn, or `None` when the turn did not come
/// through a mode-aware entry point.
pub fn current_turn_mode() -> Option<ThreadMode> {
    TURN_MODE.try_with(|mode| *mode).ok()
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
