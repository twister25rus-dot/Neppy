//! Whether the user's test command may run.
//!
//! The checks a cron shell job gets (`cron::scheduler::run_job_command`): the
//! tier must allow acting, the rate limit must not be hit, the command must
//! pass `is_command_allowed`, no path argument may be forbidden, and the action
//! budget is charged. On top of that the harness gate's verdict is applied
//! *without* an approval step, because a task runs unattended: a command the
//! gate would stop to ask about (`Prompt`) is refused in Supervised rather than
//! quietly treated as approved. Only the Full tier runs a `Prompt`-class
//! command (network, installs).
//!
//! Two deliberate differences from cron, both stricter: a command classified
//! `Destructive` is refused in *every* tier, Full included (cron's Full tier
//! would run it after an approval prompt, and a task cannot be prompted). The
//! classifier sees the outer command only, so a destructive command wrapped in
//! `sh -c '…'` is classed by its wrapper; in Full that runs, as it would for
//! cron. The command text is always the user's, never the model's; and
//! relative path arguments are resolved against the project root the command
//! runs in, not the agent's action directory.
//!
//! `check_command` is the budget-free half, used when a task is accepted so a
//! bad command is rejected up front. `authorize_run` adds the rate limit and
//! charges the budget, and is called again each time the command is about to
//! run, because the policy can change while a task waits.

use std::path::Path;

use crate::neppy::security::policy::{AutonomyLevel, CommandClass, GateDecision};
use crate::neppy::security::SecurityPolicy;

fn is_env_assignment(word: &str) -> bool {
    word.contains('=')
        && word
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
}

fn strip_wrapping_quotes(token: &str) -> &str {
    token.trim_matches(|c| c == '"' || c == '\'')
}

/// The first path-looking argument the policy forbids. A copy of the cron
/// scheduler's private helper of the same name, plus one addition: a relative
/// path argument is also judged where it lands under `cwd`, the directory the
/// command runs in. The policy's own check resolves relative paths against the
/// action directory, which is not where this command runs.
pub(crate) fn forbidden_path_argument(
    security: &SecurityPolicy,
    command: &str,
    cwd: &Path,
) -> Option<String> {
    let mut normalized = command.to_string();
    for sep in ["&&", "||"] {
        normalized = normalized.replace(sep, "\x00");
    }
    for sep in ['\n', ';', '|'] {
        normalized = normalized.replace(sep, "\x00");
    }
    for segment in normalized.split('\x00') {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let mut idx = 0;
        while idx < tokens.len() && is_env_assignment(tokens[idx]) {
            idx += 1;
        }
        if idx >= tokens.len() {
            continue;
        }
        // Skip the executable token.
        idx += 1;
        for token in &tokens[idx..] {
            let candidate = strip_wrapping_quotes(token);
            if candidate.is_empty() || candidate.starts_with('-') || candidate.contains("://") {
                continue;
            }
            let looks_like_path = candidate.starts_with('/')
                || candidate.starts_with("./")
                || candidate.starts_with("../")
                || candidate.starts_with("~/")
                || candidate.contains('/');
            if looks_like_path && !security.is_path_string_allowed(candidate) {
                return Some(candidate.to_string());
            }
            if looks_like_path && !Path::new(candidate).is_absolute() && !candidate.starts_with('~')
            {
                let landing = cwd.join(candidate);
                if !security.is_path_string_allowed(&landing.to_string_lossy())
                    || super::guards::forbidden_hit(security, &landing, false).is_some()
                {
                    return Some(candidate.to_string());
                }
            }
        }
    }
    None
}

/// Whether the policy lets this command run at all, ignoring the rate limit.
pub(crate) fn check_command(
    policy: &SecurityPolicy,
    command: &str,
    cwd: &Path,
) -> std::result::Result<(), String> {
    if !policy.can_act() {
        return Err("the autonomy tier is read-only; commands are not allowed".into());
    }
    if !policy.is_command_allowed(command) {
        return Err("the command is not on the autonomy policy's allow list".into());
    }
    let class = policy.classify_command(command);
    if class == CommandClass::Destructive {
        return Err("a destructive command cannot be used as a test command".into());
    }
    if let Some(path) = forbidden_path_argument(policy, command, cwd) {
        return Err(format!("forbidden path argument: {path}"));
    }
    match policy.gate_decision(class) {
        GateDecision::Block => Err(format!(
            "blocked by the autonomy tier (command class {class:?})"
        )),
        GateDecision::Prompt if policy.autonomy != AutonomyLevel::Full => Err(format!(
            "a {class:?}-class command needs approval and a task cannot ask for it; \
             the Supervised tier refuses it (use the Full tier, or a read-only command)"
        )),
        GateDecision::Prompt | GateDecision::Allow => Ok(()),
    }
}

/// [`check_command`], then the rate limit and the action budget, exactly as a
/// cron shell job is gated just before it spawns.
pub(crate) fn authorize_run(
    policy: &SecurityPolicy,
    command: &str,
    cwd: &Path,
) -> std::result::Result<(), String> {
    check_command(policy, command, cwd)?;
    if policy.is_rate_limited() {
        return Err("rate limit exceeded".into());
    }
    if !policy.record_action() {
        return Err("action budget exhausted".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "command_policy_tests.rs"]
mod tests;
