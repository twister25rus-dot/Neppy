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
/// A denial needs both halves: the code can start a process (it imports
/// `child_process`, or uses `execa` / `zx`), and a string literal holds a
/// publish-shaped command. No call has to be recognised, so
/// `promisify(exec)`, `const run = execSync` and `x.exec(...)` are all seen;
/// what matters is the literal. See [`literals_publish`].
pub(super) fn node_source_decision(source: &str) -> DebugCommandDecision {
    if !can_start_processes(source) {
        return Allow;
    }
    if literals_publish(&string_literals(source)) || arrays_publish(source) {
        policy_release::deny()
    } else {
        Allow
    }
}

/// The source imports `child_process` (any spelling: `node:child_process`,
/// `import`, `require`, dynamic `import()`), or uses `execa` / `zx` / `$\``.
fn can_start_processes(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.contains("child_process")
        || lower.contains("$`")
        || lower
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .any(|w| w == "execa" || w == "zx")
}

/// A quoted string of the source, and whether it sits inside a file-reading
/// call (`readFileSync('scripts/release-neppy.sh')`), where a script name is
/// data rather than something to run.
struct Literal {
    text: String,
    reads_file: bool,
}

/// Identifiers that, as the enclosing call of a literal, mean the literal is
/// read, written or inspected rather than executed.
fn is_file_call(ident: &str) -> bool {
    let l = ident.to_ascii_lowercase();
    [
        "read", "write", "copy", "append", "stat", "access", "exists", "unlink", "rename",
        "readdir", "mkdir", "rm",
    ]
    .iter()
    .any(|p| l.contains(p))
}

/// True when some call enclosing position `at` of `chars` is a file call.
fn inside_file_call(chars: &[char], at: usize) -> bool {
    let start = at.saturating_sub(400);
    let mut depth = 0usize;
    let mut i = at;
    while i > start {
        i -= 1;
        match chars[i] {
            ')' => depth += 1,
            '(' if depth > 0 => depth -= 1,
            '(' => {
                let ident: String = chars[start..i]
                    .iter()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '$'))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                if is_file_call(&ident) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// The quoted strings of `source` in order (single, double and backtick;
/// single and double quotes end at a newline). Comments and regex literals are
/// not parsed: a stray quote only mis-pairs literals, never hides one that
/// follows a closed pair.
fn string_literals(source: &str) -> Vec<Literal> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let q = chars[i];
        let open = i;
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
            out.push(Literal {
                text: lit,
                reads_file: inside_file_call(&chars, open),
            });
        }
    }
    out
}

const READ_PROGRAMS: &[&str] = &[
    "cat", "head", "tail", "less", "more", "grep", "rg", "sed", "awk", "wc", "cp", "mv", "ls",
    "stat", "file", "shasum", "diff", "bat", "open", "code",
];
const SCRIPT_NAME: &str = "release-neppy.sh";

fn base(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// Whether the command `words` publishes: the shell policy's verdict after
/// stripping wrappers (program name case-insensitive: `GH release`), looking
/// inside `sh|bash|zsh|dash|ksh -c '<inner>'` strings.
fn command_publishes(words: &[String], depth: usize) -> bool {
    let mut w = super::strip_wrappers(words).to_vec();
    let Some(first) = w.first_mut() else {
        return false;
    };
    *first = first.to_ascii_lowercase();
    if policy_release::publishes_release(&w, &[]) {
        return true;
    }
    let prog = base(&w[0]);
    if depth < 4 && matches!(prog, "sh" | "bash" | "zsh" | "dash" | "ksh" | "eval") {
        let inner = if prog == "eval" {
            Some(w[1..].join(" "))
        } else {
            w.iter()
                .position(|a| a.starts_with('-') && !a.starts_with("--") && a.ends_with('c'))
                .and_then(|i| w.get(i + 1).cloned())
        };
        if let Some(Ok(parsed)) = inner.map(|s| shell_parse::parse(&s)) {
            return parsed
                .segments
                .iter()
                .any(|seg| command_publishes(seg, depth + 1));
        }
    }
    false
}

/// True when the literals hold a publish-shaped command, in two shapes:
///
/// * a literal that is a whole command line (`gh release create v1`,
///   `cd x && pnpm release`, `sh -c "gh release create v1"`), judged segment
///   by segment by the shell rules;
/// * an argv spread over literals (`spawn('gh', ['release', 'create'])`,
///   `spawn('bash', ['scripts/release-neppy.sh'])`): every single-word literal
///   starts a window of the literals that follow it.
///
/// Literals inside a file call are data and never start anything, so reading
/// the release script stays allowed.
fn literals_publish(all: &[Literal]) -> bool {
    let lits: Vec<&str> = all
        .iter()
        .filter(|l| !l.reads_file)
        .map(|l| l.text.as_str())
        .collect();
    for (k, lit) in lits.iter().enumerate() {
        let Ok(parsed) = shell_parse::parse(lit) else {
            continue;
        };
        let multi_word = lit.split_whitespace().nth(1).is_some();
        if multi_word && parsed.segments.iter().any(|w| command_publishes(w, 0)) {
            return true;
        }
        if !multi_word && !lit.is_empty() {
            // `spawn('cat', ['scripts/release-neppy.sh'])` reads the script.
            let reads = k > 0
                && READ_PROGRAMS.contains(&base(lits[k - 1]))
                && lits[k - 1].split_whitespace().nth(1).is_none();
            if base(lit).eq_ignore_ascii_case(SCRIPT_NAME) {
                if reads {
                    continue;
                }
                return true;
            }
            let window: Vec<String> = lits[k..].iter().take(17).map(|s| s.to_string()).collect();
            if command_publishes(&window, 0) {
                return true;
            }
        }
    }
    false
}

/// `const args = ['release', 'create', 'v1']; spawnSync('gh', args)`: when a
/// literal is exactly `gh`, any string array in the source is judged as the
/// argv of a `gh` call.
fn arrays_publish(source: &str) -> bool {
    use std::sync::OnceLock;
    static ARRAY: OnceLock<regex::Regex> = OnceLock::new();
    static ITEM: OnceLock<regex::Regex> = OnceLock::new();
    let lits = string_literals(source);
    if !lits
        .iter()
        .any(|l| base(&l.text).eq_ignore_ascii_case("gh"))
    {
        return false;
    }
    let quoted = r#"(?:'[^'\n]*'|"[^"\n]*"|`[^`]*`)"#;
    let array = ARRAY.get_or_init(|| {
        regex::Regex::new(&format!(r"\[\s*{quoted}(?:\s*,\s*{quoted})*\s*,?\s*\]"))
            .expect("static array regex")
    });
    let item = ITEM.get_or_init(|| regex::Regex::new(quoted).expect("static item regex"));
    array.find_iter(source).any(|m| {
        let args: Vec<String> = item
            .find_iter(m.as_str())
            .map(|i| {
                let t = i.as_str();
                t[1..t.len() - 1].to_string()
            })
            .collect();
        policy_release::gh_publishes(&args)
    })
}

#[cfg(test)]
#[path = "policy_exec_tests.rs"]
mod tests;
