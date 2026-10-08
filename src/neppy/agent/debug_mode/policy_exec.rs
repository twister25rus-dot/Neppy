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
//! analyse. It denies the evident ways of publishing from JS: code that imports
//! `child_process` and calls a starter (`exec(`, `execSync(`, `spawn(`, ...)
//! while a string literal holds a publish-shaped command (the shell rules'
//! verdict on it, or a `gh` argv spread over literals) or names the release
//! script. Reading the script or running a read-only `gh` call is allowed. Code that builds the command at run
//! time (string concatenation, decoding, reading it from a file it wrote) is
//! beyond this check; the approval gate and the tier still apply on top.
//!
//! Both gates read the ambient Debug turn and are `Allow` everywhere else, so a
//! normal chat turn behaves exactly as before.

use super::{check_debug_command_with, policy_release, DebugCommandDecision};
use crate::neppy::agent::debug_mode::{shell_parse, turn};
use DebugCommandDecision::Allow;

/// Largest script file `node_exec` hands to the source scan.
const MAX_SCRIPT_SCAN_BYTES: u64 = 1024 * 1024;

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
///
/// A denial needs both halves: the code can start a process (it mentions
/// `child_process` and calls `exec` / `execSync` / `spawn` / `spawnSync` /
/// `execFile` / `execFileSync` / `fork`), and a string literal holds a
/// publish-shaped command (see [`literals_publish`]). Reading the release
/// script, a regex `.exec(`, or a read-only `gh run list` is not a publish.
pub(super) fn node_source_decision(source: &str) -> DebugCommandDecision {
    let lower = source.to_ascii_lowercase();
    if !lower.contains("child_process") || !calls_a_spawner(&lower) {
        return Allow;
    }
    if literals_publish(&string_literals(source)) {
        policy_release::deny()
    } else {
        Allow
    }
}

const PROCESS_OBJECTS: &[&str] = &["cp", "child_process", "childprocess", "child", "proc"];

/// True when `source` (lower-cased) calls one of the child_process starters.
/// A bare `exec(` counts; `x.exec(` counts only when `x` names a child_process
/// object (a regex or other object's `.exec(` is not a spawn).
fn calls_a_spawner(source: &str) -> bool {
    use std::sync::OnceLock;
    static CALL: OnceLock<regex::Regex> = OnceLock::new();
    let re = CALL.get_or_init(|| {
        regex::Regex::new(r"\b(exec|execsync|spawn|spawnsync|execfile|execfilesync|fork)\s*\(")
            .expect("static spawner regex")
    });
    re.captures_iter(source).any(|c| {
        let m = c.get(0).expect("whole match");
        let before = source[..m.start()].trim_end();
        if &c[1] != "exec" || !before.ends_with('.') {
            return true;
        }
        let object = before[..before.len() - 1].trim_end();
        // `require('child_process').exec(...)`
        if ["child_process')", "child_process\")", "child_process`)"]
            .iter()
            .any(|t| object.ends_with(t))
        {
            return true;
        }
        let obj: String = object
            .chars()
            .rev()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '$'))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        PROCESS_OBJECTS.contains(&obj.as_str())
    })
}

/// The quoted strings of `source` in order (single, double and backtick;
/// single and double quotes end at a newline). Comments and regex literals are
/// not parsed: a stray quote only mis-pairs literals, never hides one that
/// follows a closed pair.
fn string_literals(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let q = chars[i];
        i += 1;
        if !matches!(q, '\'' | '"' | '`') {
            continue;
        }
        let mut lit = String::new();
        let mut closed = false;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            if c == '\\' {
                if let Some(n) = chars.get(i) {
                    lit.push(*n);
                    i += 1;
                }
            } else if c == q {
                closed = true;
                break;
            } else if c == '\n' && q != '`' {
                break;
            } else {
                lit.push(c);
            }
        }
        if closed {
            out.push(lit);
        }
    }
    out
}

/// True when the literals hold a publish-shaped command: the release script
/// by name; a shell command line that [`policy_release::publishes_release`]
/// denies (`gh release create`, `pnpm release`, `bash scripts/release-...`);
/// or an argv spread over literals (`spawn('gh', ['release', 'create'])`),
/// checked by running the `gh` rules on the words that follow each `gh`.
fn literals_publish(lits: &[String]) -> bool {
    if lits
        .iter()
        .any(|l| l.to_ascii_lowercase().contains("release-neppy"))
    {
        return true;
    }
    for lit in lits {
        let Ok(parsed) = shell_parse::parse(lit) else {
            continue;
        };
        // Program names are case-insensitive on macOS and Windows: `GH release`.
        let hit = parsed.segments.iter().any(|w| {
            let mut w = super::strip_wrappers(w).to_vec();
            if let Some(first) = w.first_mut() {
                *first = first.to_ascii_lowercase();
            }
            policy_release::publishes_release(&w, &[])
        });
        if hit {
            return true;
        }
    }
    let words: Vec<String> = lits
        .iter()
        .flat_map(|l| l.split_whitespace().map(str::to_string))
        .collect();
    words.iter().enumerate().any(|(i, w)| {
        w.eq_ignore_ascii_case("gh")
            && policy_release::gh_publishes(&words[i + 1..(i + 17).min(words.len())])
    })
}

#[cfg(test)]
#[path = "policy_exec_tests.rs"]
mod tests;
