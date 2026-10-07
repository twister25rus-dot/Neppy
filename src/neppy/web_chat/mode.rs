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
//! Only the `orchestrator` agent has the chat/orchestration modes. A profile
//! that routes the thread to some other agent keeps that agent's own surface
//! untouched.
//!
//! **Debug mode is different: it picks the agent.** A thread persisted in
//! Debug mode runs `debug_agent` whatever its profile says
//! ([`target_agent_id`]), and `debug_agent` runs *only* on such a thread
//! ([`debug_agent_allowed`]). The repo scope and per-turn bookkeeping live in
//! `agent::debug_mode::turn`; this module only decides the routing.

use crate::neppy::agent::Agent;
use crate::neppy::threads::mode::ThreadMode;

/// The agent that implements both modes.
pub(crate) const MODE_AWARE_AGENT_ID: &str = "orchestrator";

/// The agent every Debug-mode turn runs.
pub(crate) use crate::neppy::agent::debug_mode::turn::DEBUG_AGENT_ID;

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
const DEBUG_ADDENDUM: &str = include_str!("mode_prompts/debug.md");

/// The mode `target_agent_id` should run this turn in, or `None` when the agent
/// does not implement modes.
pub(crate) fn effective_mode(target_agent_id: &str, persisted: ThreadMode) -> Option<ThreadMode> {
    if target_agent_id == DEBUG_AGENT_ID {
        return (persisted == ThreadMode::Debug).then_some(ThreadMode::Debug);
    }
    (target_agent_id == MODE_AWARE_AGENT_ID).then_some(persisted)
}

/// The agent a turn runs: `debug_agent` on a Debug-mode thread regardless of
/// the profile's choice (`profile_agent_id`), otherwise the profile's choice,
/// byte-identical to the pre-Debug routing.
pub(crate) fn target_agent_id(profile_agent_id: String, persisted: ThreadMode) -> String {
    if persisted == ThreadMode::Debug {
        DEBUG_AGENT_ID.to_string()
    } else {
        profile_agent_id
    }
}

/// [`target_agent_id`] plus the refusal for a profile that names `debug_agent`
/// on a thread that is not in Debug mode.
pub(crate) fn resolve_target_agent(
    profile_agent_id: String,
    persisted: ThreadMode,
) -> Result<String, String> {
    let target = target_agent_id(profile_agent_id, persisted);
    if !debug_agent_allowed(&target, persisted) {
        return Err("The debug agent is only available on a thread in Debug mode; switch this thread to Debug mode to use it.".to_string());
    }
    Ok(target)
}

/// Whether the current task is already inside a Debug-mode turn scope.
pub(crate) fn in_debug_turn() -> bool {
    crate::neppy::agent::debug_mode::turn::current().is_some()
}

/// A profile's dedicated workspace outranks the per-turn root as the session's
/// default cwd (`derive_profile_workspace_descriptor` wins over
/// `derive_turn_workspace_descriptor`), so a Debug turn never takes that
/// opt-in: it must work in the project repository.
pub(crate) fn prepare_profile(
    profile: &mut crate::neppy::agent::profiles::AgentProfile,
    persisted: ThreadMode,
) {
    if persisted == ThreadMode::Debug && profile.dedicated_workspace {
        log::debug!("[debug_mode] ignoring profile dedicated_workspace for a debug turn");
        profile.dedicated_workspace = false;
    }
}

/// `debug_agent` is reachable only through Debug mode: a profile that names it
/// on a non-Debug thread would run it with no repo scope and no task record.
pub(crate) fn debug_agent_allowed(target_agent_id: &str, persisted: ThreadMode) -> bool {
    target_agent_id != DEBUG_AGENT_ID || persisted == ThreadMode::Debug
}

/// System-prompt addendum for `mode`. Debug mode appends a short block derived
/// from the turn's saved `[debug_mode]` settings (repair budget, whether tests
/// and the build run, whether installs / push are allowed) when it is called
/// inside a Debug turn, which is where session construction runs.
pub(crate) fn prompt_addendum(mode: ThreadMode) -> std::borrow::Cow<'static, str> {
    use std::borrow::Cow;
    match mode {
        ThreadMode::Chat => Cow::Borrowed(CHAT_ADDENDUM.trim()),
        ThreadMode::Orchestration => Cow::Borrowed(ORCHESTRATION_ADDENDUM.trim()),
        ThreadMode::Debug => {
            let base = DEBUG_ADDENDUM.trim();
            match crate::neppy::agent::debug_mode::turn::current() {
                Some(turn) => Cow::Owned(format!(
                    "{base}\n\n{}",
                    crate::neppy::agent::debug_mode::policy::prompt_addendum(&turn.settings)
                )),
                None => Cow::Borrowed(base),
            }
        }
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
        ThreadMode::Debug => {
            // The debug agent's own definition is the whole tool surface.
            log::debug!("[mode] debug: debug_agent tool surface untouched");
        }
    }
}

#[cfg(test)]
#[path = "mode_tests.rs"]
mod tests;
