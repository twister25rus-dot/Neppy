//! Sub-agent cap-hit checkpoint summary.
//!
//! When the iteration cap is hit, summarize the run-so-far into a resumable
//! checkpoint (so the delegating agent can continue from partial progress)
//! instead of erroring. Falls back to a deterministic digest summary if the
//! summarization call fails or returns no prose.

use crate::neppy::inference::provider::UsageInfo;
use std::sync::Arc;
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ChatModel, ModelRequest};

/// A checkpoint result. `usage`, when present, is the provider usage from the
/// summary call so the caller can fold it into sub-agent token/cost accounting.
pub(super) struct SubagentCheckpointOutcome {
    pub(super) text: String,
    pub(super) usage: Option<UsageInfo>,
}

/// Sub-agent cap-hit summary: when the iteration cap is hit, summarize the
/// run-so-far into a resumable checkpoint (so the delegating agent can continue
/// from partial progress) instead of erroring. Falls back to a deterministic
/// digest summary if the summarization call fails or returns no prose.
///
/// The summary runs on a crate [`ChatModel`] (built from the turn's
/// [`TurnModelSource`](crate::neppy::agent::tinyagents::TurnModelSource) — model +
/// temperature baked in), so the checkpoint no longer names the `Provider` trait
/// (issue #4249, Phase 3 / Motion A).
pub(super) struct SubagentCheckpoint {
    pub(super) chat_model: Arc<dyn ChatModel<()>>,
    pub(super) agent_id: String,
    pub(super) max_output_tokens: u32,
}

impl SubagentCheckpoint {
    pub(super) async fn summarize_cap_hit(
        &self,
        digest: &str,
        max_iterations: usize,
    ) -> anyhow::Result<SubagentCheckpointOutcome> {
        let agent_id = &self.agent_id;
        let deterministic = format!(
            "I reached my tool-call limit ({max_iterations} steps) before finishing this task. \
             Progress so far (tool calls + results):\n{digest}\n\nThe task is incomplete — the above is \
             what I accomplished; continue from here."
        );
        let summary_input = vec![Message::user(format!(
            "You are sub-agent `{agent_id}` and reached your tool-call limit before finishing. Here are \
             the tool calls you made and their results — compile a brief progress checkpoint (what you \
             accomplished, what still remains) for the agent that delegated to you. Do not call tools.\n\n{digest}"
        ))];
        // Bounded progress-summary turn; the cap also keeps the reservation-pricing
        // pre-flight realistic (TAURI-RUST-C62). Temperature is baked into the model.
        let request = ModelRequest::new(summary_input).with_max_tokens(self.max_output_tokens);
        match self.chat_model.invoke(&(), request).await {
            Ok(resp) => {
                let usage = crate::neppy::agent::tinyagents::model::usage_info_from_response(&resp);
                let raw = resp.text();
                let (prose, _) = super::super::super::parse::parse_tool_calls(&raw);
                let text = if prose.trim().is_empty() {
                    deterministic
                } else {
                    prose
                };
                Ok(SubagentCheckpointOutcome { text, usage })
            }
            Err(e) => {
                tracing::warn!(
                    agent_id = %self.agent_id,
                    error = %e,
                    "[subagent_runner] checkpoint summary call failed — using deterministic fallback"
                );
                Ok(SubagentCheckpointOutcome {
                    text: deterministic,
                    usage: None,
                })
            }
        }
    }
}

/// The `Incomplete` reason a sub-agent reports when its wall-clock deadline
/// stopped it (M2). The async-delivery framing reads "the sub-agent timed out
/// and did not finish".
pub(super) const SUBAGENT_TIMED_OUT_REASON: &str = "timed out";

/// Deterministic `- role: content` digest of the rounds a sub-agent completed
/// before it was stopped, from the transcript snapshot (system prompt and the
/// opening prompt excluded by the caller's slice). Each entry is capped so a
/// large tool result cannot swamp the parent's context.
pub(super) fn transcript_digest(messages: &[crate::neppy::agent::messages::ChatMessage]) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for message in messages {
        if message.role == "system" || message.content.trim().is_empty() {
            continue;
        }
        let body = crate::neppy::util::truncate_with_ellipsis(message.content.trim(), 800);
        let _ = writeln!(out, "- {}: {body}", message.role);
    }
    out.trim_end().to_string()
}

/// The checkpoint a timed-out sub-agent hands back (M2): what it did before the
/// deadline, framed as incomplete so the delegating agent continues from it
/// rather than treating it as an answer.
///
/// Deliberately deterministic — no summary model call. The run is already past
/// its deadline (and, nested, close to its parent's), so a further provider call
/// would most likely be cut off too and lose the progress this exists to keep.
pub(super) fn timed_out_checkpoint(agent_id: &str, digest: &str) -> String {
    let progress = if digest.trim().is_empty() {
        "No steps completed before the deadline.".to_string()
    } else {
        format!("Progress so far:\n{digest}")
    };
    format!(
        "Sub-agent `{agent_id}` ran out of time (wall-clock limit) before finishing this task. \
         {progress}\n\nThe task is incomplete — the above is what was accomplished; continue \
         from here."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::agent::messages::ChatMessage;

    #[test]
    fn timed_out_checkpoint_keeps_the_completed_rounds() {
        let digest = transcript_digest(&[
            ChatMessage::system("you are a researcher"),
            ChatMessage::assistant("looking it up"),
            ChatMessage::user("[tool result] echoed:hi"),
        ]);
        assert!(!digest.contains("you are a researcher"));
        let text = timed_out_checkpoint("researcher", &digest);
        assert!(text.contains("ran out of time"));
        assert!(text.contains("echoed:hi"));
        assert!(text.contains("incomplete"));
    }

    #[test]
    fn timed_out_checkpoint_says_so_when_nothing_completed() {
        let text = timed_out_checkpoint("researcher", "");
        assert!(text.contains("No steps completed"));
    }
}
