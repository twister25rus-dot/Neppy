//! The Debug-turn gate for the two tools that run commands without a shell
//! string: `npm_exec` (an npm subcommand plus arguments) and `node_exec`
//! (JavaScript source or a script file).
//!
//! `npm_exec` is the `npm` branch of the shell gate: its subcommand and
//! arguments are rebuilt into a quoted command line and classified exactly as
//! `shell` would classify it, so `npm_exec install left-pad` is the dependency
//! install it is and `npm_exec run release` is the release script it is.
//!
//! `node_exec` runs arbitrary JavaScript, which a static check cannot fully
//! analyse. It denies the evident ways of publishing from JS: code that can
//! start a process (`child_process`, `spawn`, `exec`) and names the release
//! script, `gh release` / `gh workflow run`, or `gh` together with a
//! release, workflow or dispatch operand. Code that builds the command at run
//! time (string concatenation, decoding, reading it from a file it wrote) is
//! beyond this check; the approval gate and the tier still apply on top.
//!
//! Both gates read the ambient Debug turn and are `Allow` everywhere else, so a
//! normal chat turn behaves exactly as before.

use super::{check_debug_command_with, policy_release, DebugCommandDecision};
use crate::neppy::agent::debug_mode::turn;
use DebugCommandDecision::Allow;

/// Largest script file `node_exec` hands to the source scan.
const MAX_SCRIPT_SCAN_BYTES: u64 = 1024 * 1024;

const SPAWN_MARKERS: &[&str] = &["child_process", "spawn", "exec"];
const GH_RELEASE_MARKERS: &[&str] = &["release", "workflow", "dispatches", "promote"];

/// `npm <subcommand> <args...>` with every word single-quoted, so the shell
/// parser sees the same words the tool will pass to npm.
pub(super) fn npm_command_line(subcommand: &str, args: &[String]) -> String {
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    std::iter::once("npm".to_string())
        .chain(std::iter::once(quote(subcommand)))
        .chain(args.iter().map(|a| quote(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The gate for an `npm_exec` call in the ambient Debug turn.
pub fn gate_npm_exec_current(subcommand: &str, args: &[String]) -> DebugCommandDecision {
    let Some(t) = turn::current() else {
        return Allow;
    };
    let line = npm_command_line(subcommand, args);
    let d = check_debug_command_with(
        &line,
        &t.settings,
        &policy_release::wrapping_scripts(&t.root),
    );
    log::debug!(
        "[debug_mode][npm_exec] gate subcommand={subcommand} args={}",
        args.len()
    );
    d
}

/// [`gate_npm_exec_current`] for the raw tool arguments (`subcommand`, `args`),
/// for the approval hook that runs before `execute`. A call with no usable
/// subcommand is `Allow`: the tool rejects it itself.
pub fn gate_npm_exec_args_current(args: &serde_json::Value) -> DebugCommandDecision {
    let Some(sub) = args.get("subcommand").and_then(|v| v.as_str()) else {
        return Allow;
    };
    let rest: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    gate_npm_exec_current(sub.trim(), &rest)
}

/// [`gate_node_exec_current`] for a script file: scans its first
/// [`MAX_SCRIPT_SCAN_BYTES`] bytes. An unreadable file is `Allow` (the tool
/// reports it when it tries to run it).
pub fn gate_node_exec_file_current(path: &std::path::Path) -> DebugCommandDecision {
    use std::io::Read;
    if turn::current().is_none() {
        return Allow;
    }
    let mut buf = Vec::new();
    let read =
        std::fs::File::open(path).and_then(|f| f.take(MAX_SCRIPT_SCAN_BYTES).read_to_end(&mut buf));
    if read.is_err() {
        log::debug!("[debug_mode][node_exec] script not scanned: unreadable");
        return Allow;
    }
    gate_node_exec_current(&String::from_utf8_lossy(&buf))
}

/// The gate for a `node_exec` call (`source` is the inline code, or the script
/// file's text) in the ambient Debug turn.
pub fn gate_node_exec_current(source: &str) -> DebugCommandDecision {
    if turn::current().is_none() {
        return Allow;
    }
    let d = node_source_decision(source);
    log::debug!(
        "[debug_mode][node_exec] gate bytes={} denied={}",
        source.len(),
        matches!(d, DebugCommandDecision::Deny(_))
    );
    d
}

/// Pure scan of JavaScript source; see the module docs for what it catches.
pub(super) fn node_source_decision(source: &str) -> DebugCommandDecision {
    let lower = source.to_ascii_lowercase();
    if !SPAWN_MARKERS.iter().any(|m| lower.contains(m)) {
        return Allow;
    }
    // `gh` as a whole word anywhere (an argv element, `gh release ...`) plus a
    // release / workflow / dispatch word covers `gh release` and `gh workflow run`.
    let names_publish = lower.contains("release-neppy")
        || (contains_word(&lower, "gh") && GH_RELEASE_MARKERS.iter().any(|m| lower.contains(m)));
    if names_publish {
        policy_release::deny()
    } else {
        Allow
    }
}

/// `needle` as a whole word (not inside `github`, `high`, ...).
fn contains_word(haystack: &str, needle: &str) -> bool {
    haystack
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|w| w == needle)
}

#[cfg(test)]
#[path = "policy_exec_tests.rs"]
mod tests;
