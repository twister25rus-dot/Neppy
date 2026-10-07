//! Time budgets of a Debug-mode turn.
//!
//! The default turn budget (10 minutes, `agent::tinyagents`) is sized for chat.
//! Repository work does not fit in it: a cold `cargo build` or a candidate
//! validation takes minutes on its own. A Debug turn therefore gets its own,
//! larger budget (`[debug_mode].turn_timeout_secs`, 60 minutes by default), and
//! every command it runs gets a per-call deadline so a single hung process can
//! never consume the whole turn: the call fails with an error the model can act
//! on instead.
//!
//! Every function here is inert (`None` / the base value) outside a Debug turn,
//! so a normal chat turn behaves exactly as it did before.

use std::time::Duration;

use crate::neppy::config::schema::debug_mode::DebugModeConfig;

use super::turn;

/// How long one `shell` / `node_exec` / `npm_exec` call may run in a Debug turn
/// when the model did not pass its own `timeout_secs` (20 minutes: longer than a
/// cold build, far shorter than the turn). An explicit `timeout_secs` still wins.
pub const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 1200;

/// Grace the outer web-channel backstop adds on top of the harness budget, so
/// the harness always fires first and returns its checkpointed error.
const OUTER_BACKSTOP_GRACE_SECS: u64 = 300;

/// The harness wall-clock ceiling (ms) for a turn whose default is `base`.
///
/// `base == None` is the operator's explicit "no ceiling" opt-out
/// (`NEPPY_AGENT_TURN_TIMEOUT_SECS=0`) and is honoured. Otherwise a Debug turn
/// takes `settings.turn_timeout_secs` (`0` = no ceiling), never less than `base`.
pub fn turn_wall_clock_ms(base: Option<u64>, settings: &DebugModeConfig) -> Option<u64> {
    let base = base?;
    if settings.turn_timeout_secs == 0 {
        return None;
    }
    Some(base.max(settings.turn_timeout_secs.saturating_mul(1_000)))
}

/// [`turn_wall_clock_ms`] for the ambient Debug turn; `base` outside one.
pub fn current_turn_wall_clock_ms(base: Option<u64>) -> Option<u64> {
    match turn::current() {
        Some(t) => {
            let ms = turn_wall_clock_ms(base, &t.settings);
            log::debug!(
                "[debug_mode] turn budget base_ms={base:?} -> {ms:?} (turn_timeout_secs={})",
                t.settings.turn_timeout_secs
            );
            ms
        }
        None => base,
    }
}

/// The outer web-channel backstop for a Debug thread under `settings`: the
/// harness budget plus a grace window, or `None` when the turn has no ceiling.
/// `base` is the backstop a normal turn gets.
pub fn web_backstop(base: Option<Duration>, settings: &DebugModeConfig) -> Option<Duration> {
    let base = base?;
    if settings.turn_timeout_secs == 0 {
        return None;
    }
    let want = Duration::from_secs(
        settings
            .turn_timeout_secs
            .saturating_add(OUTER_BACKSTOP_GRACE_SECS),
    );
    Some(base.max(want))
}

/// The per-call deadline (seconds) a process-running tool must apply in the
/// ambient Debug turn when the model asked for none; `None` outside one.
pub fn default_tool_timeout_secs() -> Option<u64> {
    turn::current().map(|_| DEFAULT_TOOL_TIMEOUT_SECS)
}

/// The `timeout_secs` a process-running tool should honour for a call that
/// requested `requested`: in a Debug turn a missing or `0` (= "no deadline")
/// request becomes [`DEFAULT_TOOL_TIMEOUT_SECS`], so one wedged process cannot
/// eat the whole turn; an explicit positive request is kept. Outside a Debug
/// turn the request is returned unchanged.
pub fn effective_tool_timeout(requested: Option<u64>) -> Option<u64> {
    match (requested, default_tool_timeout_secs()) {
        (None | Some(0), Some(default)) => {
            log::debug!("[debug_mode] no tool deadline requested; applying {default}s");
            Some(default)
        }
        _ => requested,
    }
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
