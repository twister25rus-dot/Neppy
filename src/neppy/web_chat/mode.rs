//! How a thread's operating mode shapes a chat turn.
//!
//! The mode (`threads::mode::ThreadMode`) is persisted with the thread; this
//! module is the single place that turns it into behaviour:
//!
//! * **Tool surface.** Chat mode hides the multi-agent fleet tools for the turn
//!   through [`Agent::hide_tools`] — the same per-session seam the flows
//!   builder and cron use — so a hidden tool is denied at the call boundary,
//!   not merely absent from the prompt. Orchestration mode hides nothing: the
//!   supervisor belt (`spawn_*`, `wait_subagent`, `steer_subagent`,
//!   `close_subagent`, `list_subagents`, `continue_subagent`) is whatever the
//!   orchestrator definition names.
//! * **Prompt.** A short addendum is appended to the system prompt through the
//!   profile suffix lane (see `session::build_session_agent`).
//! * **Delegate blocking.** In chat mode the `delegate_*` capability helpers run
//!   inline (see `agent::orchestration::tools::dispatch`) so no background
//!   worker ever surfaces; the ambient mode they read is scoped by
//!   `threads::mode::with_turn_mode` around the turn.
//!
//! Only the `orchestrator` agent has the two modes. A profile that routes the
//! thread to some other agent keeps that agent's own surface untouched.

use crate::neppy::agent::Agent;
use crate::neppy::threads::mode::ThreadMode;

/// The agent that implements both modes.
pub(crate) const MODE_AWARE_AGENT_ID: &str = "orchestrator";

/// Tools hidden for a chat-mode turn: everything that starts, inspects,
/// steers, waits on or closes a fleet of async workers.
///
/// Deliberately **not** listed:
/// * the `delegate_*` capability helpers (MCP, memory, skills, integrations,
///   setup) — they stay, and run blocking in this mode;
/// * `continue_subagent` — a blocking helper such as `delegate_setup_mcp_server`
///   can still pause on `ask_user_clarification`, and `continue_subagent` is the
///   only way to resume that exact checkpoint. Hiding it reintroduces the
///   re-delegate-and-ask-again loop (#4291).
pub(crate) const CHAT_HIDDEN_TOOLS: &[&str] = &[
    "spawn_async_subagent",
    "spawn_parallel_agents",
    "list_subagents",
    "steer_subagent",
    "close_subagent",
    "wait_subagent",
    "wait",
    "wait_loop",
];

/// Tools only the supervisor mode surfaces. They are named in the orchestrator
/// definition so that Orchestration mode has them; chat mode hides them via
/// [`CHAT_HIDDEN_TOOLS`].
#[cfg(test)]
pub(crate) const ORCHESTRATION_ONLY_TOOLS: &[&str] = &[
    "spawn_parallel_agents",
    "steer_subagent",
    "close_subagent",
    "wait_subagent",
];

const CHAT_ADDENDUM: &str = include_str!("mode_prompts/chat.md");
const ORCHESTRATION_ADDENDUM: &str = include_str!("mode_prompts/orchestration.md");

/// The mode `target_agent_id` should run this turn in, or `None` when the agent
/// does not implement modes.
pub(crate) fn effective_mode(target_agent_id: &str, persisted: ThreadMode) -> Option<ThreadMode> {
    (target_agent_id == MODE_AWARE_AGENT_ID).then_some(persisted)
}

/// System-prompt addendum for `mode`.
pub(crate) fn prompt_addendum(mode: ThreadMode) -> &'static str {
    match mode {
        ThreadMode::Chat => CHAT_ADDENDUM.trim(),
        ThreadMode::Orchestration => ORCHESTRATION_ADDENDUM.trim(),
    }
}

/// Applies the tool-surface half of `mode` to a freshly built session agent.
pub(crate) fn apply_to_agent(agent: &mut Agent, mode: ThreadMode) {
    match mode {
        ThreadMode::Chat => {
            agent.hide_tools(CHAT_HIDDEN_TOOLS);
            log::debug!(
                "[mode] chat: hid {} fleet tools for this session ({})",
                CHAT_HIDDEN_TOOLS.len(),
                CHAT_HIDDEN_TOOLS.join(",")
            );
        }
        ThreadMode::Orchestration => {
            log::debug!("[mode] orchestration: full delegation surface exposed");
        }
    }
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
