//! Debug Mode enforcement: what the saved `[debug_mode]` settings allow.
//!
//! Everything here is a pure function of a [`DebugModeConfig`] except the two
//! `*_current_turn` seams, which read the ambient Debug turn and are inert
//! (`Allow` / `None`) everywhere else — a normal chat turn behaves exactly as it
//! did before this module existed.
//!
//! The shell gate classifies *every* simple command a string could run
//! (`&&`, `;`, `|`, subshells, `sh -c`, `xargs`, `env`, ...). Anything it cannot
//! parse, or that hides a substitution inside quotes, asks instead of allowing.

use crate::neppy::config::schema::debug_mode::DebugModeConfig;

use super::shell_parse;

/// What the Debug-turn command gate decided about one shell string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugCommandDecision {
    Allow,
    /// Needs a human yes through the normal approval gate.
    Ask(String),
    /// Refused outright; the reason goes back to the model as the tool error.
    Deny(String),
}

impl DebugCommandDecision {
    fn rank(&self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Ask(_) => 1,
            Self::Deny(_) => 2,
        }
    }
    fn worst(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

use DebugCommandDecision::{Allow, Ask, Deny};

const MAX_DEPTH: usize = 3;

/// The error a disabled Debug Mode gives a turn.
pub fn ensure_enabled(cfg: &DebugModeConfig) -> Result<(), String> {
    if cfg.enabled {
        Ok(())
    } else {
        log::warn!("[debug_mode] turn refused: debug_mode.enabled=false");
        Err(
            "Debug mode is turned off. Enable it in Settings > Debug mode, then try again."
                .to_string(),
        )
    }
}

/// `Err` with the user-facing reason when committing is not allowed.
pub fn ensure_commit_allowed(cfg: &DebugModeConfig) -> Result<(), String> {
    if cfg.allow_git_commit {
        Ok(())
    } else {
        Err(
            "commit refused: Debug mode has git commits turned off (allow_git_commit=false)."
                .to_string(),
        )
    }
}

// ── shell command gate ──────────────────────────────────────────────────

/// Classifies a shell string for a Debug turn under `cfg`.
pub fn check_debug_command(cmd: &str, cfg: &DebugModeConfig) -> DebugCommandDecision {
    check_debug_command_with(cmd, cfg, &[])
}

/// [`check_debug_command`] plus the names of package scripts that wrap a release.
pub(super) fn check_debug_command_with(
    cmd: &str,
    cfg: &DebugModeConfig,
    release_scripts: &[String],
) -> DebugCommandDecision {
    let d = check_inner(cmd, cfg, 0, release_scripts);
    log::debug!(
        "[debug_mode] command gate decision={}",
        match &d {
            Allow => "allow",
            Ask(_) => "ask",
            Deny(_) => "deny",
        }
    );
    d
}

/// The gate for the ambient Debug turn; `Allow` when there is none.
pub fn gate_current_command(cmd: &str) -> DebugCommandDecision {
    match super::turn::current() {
        Some(t) => {
            check_debug_command_with(cmd, &t.settings, &policy_release::wrapping_scripts(&t.root))
        }
        None => Allow,
    }
}

/// Reason a `git_operations` call is refused in the ambient Debug turn.
pub fn git_operation_denied(operation: &str) -> Option<String> {
    let t = super::turn::current()?;
    match operation {
        "push" if !t.settings.allow_git_push => Some(deny_push()),
        "commit" if !t.settings.allow_git_commit => Some(deny_commit()),
        _ => None,
    }
}

fn deny_push() -> String {
    "git push is not allowed in Debug mode (allow_git_push=false).".to_string()
}
fn deny_commit() -> String {
    "git commit is not allowed in Debug mode (allow_git_commit=false).".to_string()
}

fn check_inner(
    cmd: &str,
    cfg: &DebugModeConfig,
    depth: usize,
    rel: &[String],
) -> DebugCommandDecision {
    let parsed = match shell_parse::parse(cmd) {
        Ok(p) => p,
        Err(()) => return Ask("the command could not be parsed safely".to_string()),
    };
    let mut decision = Allow;
    for words in &parsed.segments {
        decision = decision.worst(check_words(words, cfg, depth, rel));
        if matches!(decision, Deny(_)) {
            return decision;
        }
    }
    if parsed.complex {
        decision = decision.worst(Ask("a command substitution inside quotes".to_string()));
    }
    decision
}

fn base(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

fn is_assignment(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        !k.is_empty()
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Drops leading assignments and transparent wrappers; returns the words of the
/// command that actually runs (possibly empty).
pub(super) fn strip_wrappers(mut w: &[String]) -> &[String] {
    loop {
        let Some(first) = w.first() else { return w };
        if is_assignment(first)
            || matches!(first.as_str(), "{" | "}" | "!" | "do" | "then" | "else")
        {
            w = &w[1..];
            continue;
        }
        match base(first) {
            "command" | "builtin" | "exec" | "time" | "nohup" | "setsid" => {
                w = skip_flags(&w[1..]);
            }
            "env" => {
                let mut rest = &w[1..];
                while let Some(x) = rest.first() {
                    if x.starts_with('-') || is_assignment(x) {
                        rest = &rest[1..];
                    } else {
                        break;
                    }
                }
                w = rest;
            }
            "nice" => {
                let mut rest = &w[1..];
                if rest.first().is_some_and(|x| x == "-n") {
                    rest = rest.get(2..).unwrap_or(&[]);
                }
                w = skip_flags(rest);
            }
            "timeout" => {
                let rest = skip_flags(&w[1..]);
                w = rest.get(1..).unwrap_or(&[]); // the duration
            }
            "xargs" => {
                let mut rest = &w[1..];
                while let Some(x) = rest.first() {
                    if !x.starts_with('-') {
                        break;
                    }
                    let takes = matches!(
                        x.as_str(),
                        "-I" | "-n" | "-P" | "-d" | "-L" | "-s" | "-E" | "-a"
                    );
                    rest = &rest[if takes { 2.min(rest.len()) } else { 1 }..];
                }
                w = rest;
            }
            _ => return w,
        }
    }
}

fn skip_flags(mut w: &[String]) -> &[String] {
    while w.first().is_some_and(|x| x.starts_with('-')) {
        w = &w[1..];
    }
    w
}

fn check_words(
    words: &[String],
    cfg: &DebugModeConfig,
    depth: usize,
    rel: &[String],
) -> DebugCommandDecision {
    let w = strip_wrappers(words);
    let Some(prog) = w.first() else { return Allow };
    // Never configurable: no setting lets the agent publish a release.
    if policy_release::publishes_release(w, rel) {
        return policy_release::deny();
    }
    let args = &w[1..];
    let prog = base(prog);
    let sys = |what: &str| -> DebugCommandDecision {
        if cfg.allow_system_commands {
            Allow
        } else {
            Deny(format!(
                "`{what}` is a system command and is not allowed in Debug mode (allow_system_commands=false)."
            ))
        }
    };
    let dep = |what: &str| -> DebugCommandDecision {
        if cfg.allow_dependency_install {
            Allow
        } else {
            Deny(format!(
                "Installing dependencies (`{what}`) is not allowed in Debug mode (allow_dependency_install=false). Use an existing dependency or ask the user to enable it."
            ))
        }
    };
    let danger = |what: &str| -> DebugCommandDecision {
        if cfg.dangerous_commands_require_confirmation {
            Ask(format!("`{what}` is destructive and needs confirmation."))
        } else {
            Allow
        }
    };

    match prog {
        "sudo" | "su" | "doas" | "launchctl" | "systemctl" | "diskutil" | "chown" | "dd"
        | "mount" | "umount" | "shutdown" | "reboot" => sys(prog),
        p if p == "mkfs" || p.starts_with("mkfs.") => sys("mkfs"),
        "defaults"
            if args
                .iter()
                .any(|a| matches!(a.as_str(), "write" | "delete" | "import")) =>
        {
            sys("defaults write")
        }
        "chmod" | "chgrp"
            if args.iter().any(|a| {
                (a.starts_with('-') && !a.starts_with("--") && a.contains('R'))
                    || a == "--recursive"
            }) && args
                .iter()
                .any(|a| a.starts_with('/') || a.starts_with('~')) =>
        {
            sys("chmod -R on an absolute path")
        }
        "sh" | "bash" | "zsh" | "dash" | "ksh" => match args
            .iter()
            .position(|a| a.starts_with('-') && !a.starts_with("--") && a.ends_with('c'))
            .and_then(|i| args.get(i + 1))
        {
            Some(script) if depth < MAX_DEPTH => check_inner(script, cfg, depth + 1, rel),
            Some(s) if policy_release::mentions_script(s) => policy_release::deny(),
            Some(_) => Ask("deeply nested shell invocation".to_string()),
            None => Allow,
        },
        "eval" if depth < MAX_DEPTH => check_inner(&args.join(" "), cfg, depth + 1, rel),
        "eval" if policy_release::mentions_script(&args.join(" ")) => policy_release::deny(),
        "eval" => Ask("deeply nested eval".to_string()),
        "rm" if has_recursive(args) => danger("rm -r"),
        "git" => check_git(args, cfg),
        "npm" | "pnpm" | "yarn" | "bun" => check_js_pm(prog, args, &dep),
        "cargo" => match first_subcommand(args, true) {
            Some("add") => dep("cargo add"),
            Some("install") => dep("cargo install"),
            _ => Allow,
        },
        "pip" | "pip3" => check_pip(args, &dep),
        p if p.starts_with("pip3.") => check_pip(args, &dep),
        "python" | "python3"
            if args.first().is_some_and(|a| a == "-m")
                && args.get(1).is_some_and(|a| a == "pip") =>
        {
            check_pip(&args[2..], &dep)
        }
        "uv" => match first_subcommand(args, false) {
            Some("add") => dep("uv add"),
            Some("pip") if args.iter().any(|a| a == "install") => check_pip(
                &args[args.iter().position(|a| a == "pip").unwrap_or(0) + 1..],
                &dep,
            ),
            _ => Allow,
        },
        "poetry" if first_subcommand(args, false) == Some("add") => dep("poetry add"),
        "brew" if matches!(first_subcommand(args, false), Some("install" | "reinstall")) => {
            dep("brew install")
        }
        "go" if first_subcommand(args, false) == Some("get") => dep("go get"),
        _ => Allow,
    }
}

fn has_recursive(args: &[String]) -> bool {
    args.iter().any(|a| {
        a == "--recursive" || (a.starts_with('-') && !a.starts_with("--") && a.contains(['r', 'R']))
    })
}

fn first_subcommand(args: &[String], skip_plus: bool) -> Option<&str> {
    args.iter()
        .find(|a| !(a.starts_with('-') || (skip_plus && a.starts_with('+'))))
        .map(String::as_str)
}

fn check_js_pm(
    prog: &str,
    args: &[String],
    dep: &dyn Fn(&str) -> DebugCommandDecision,
) -> DebugCommandDecision {
    // Skip flags and the values of the flags that take one.
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if !a.starts_with('-') {
            break;
        }
        let takes = matches!(
            a,
            "--filter" | "-F" | "-C" | "--dir" | "--prefix" | "--cwd" | "-w" | "--workspace"
        );
        i += if takes { 2 } else { 1 };
    }
    let Some(sub) = args.get(i).map(String::as_str) else {
        return Allow;
    };
    let rest = &args[i + 1..];
    let has_pkg = rest.iter().any(|a| !a.starts_with('-'));
    match sub {
        "add" => dep(&format!("{prog} add")),
        "install" | "i" if has_pkg => dep(&format!("{prog} {sub} <package>")),
        _ => Allow,
    }
}

fn check_pip(args: &[String], dep: &dyn Fn(&str) -> DebugCommandDecision) -> DebugCommandDecision {
    let Some(pos) = args.iter().position(|a| a == "install") else {
        return Allow;
    };
    let mut rest = args[pos + 1..].iter();
    let mut has_pkg = false;
    while let Some(a) = rest.next() {
        if matches!(a.as_str(), "-r" | "--requirement" | "-c" | "--constraint") {
            rest.next(); // restoring from a requirements file
        } else if !a.starts_with('-') {
            has_pkg = true;
        }
    }
    if has_pkg {
        dep("pip install <package>")
    } else {
        Allow
    }
}

fn check_git(args: &[String], cfg: &DebugModeConfig) -> DebugCommandDecision {
    // Skip git's global options to find the subcommand.
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if !a.starts_with('-') {
            break;
        }
        let takes = matches!(
            a,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
        );
        i += if takes { 2 } else { 1 };
    }
    let Some(sub) = args.get(i).map(String::as_str) else {
        return Allow;
    };
    let rest = &args[i + 1..];
    let flag = |long: &str| rest.iter().any(|a| a == long);
    let short_has = |c: char| {
        rest.iter()
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains(c))
    };
    let danger = |what: &str| {
        if cfg.dangerous_commands_require_confirmation {
            Ask(format!("`{what}` is destructive and needs confirmation."))
        } else {
            Allow
        }
    };
    match sub {
        "push" => {
            if !cfg.allow_git_push {
                return Deny(deny_push());
            }
            let force = flag("--force")
                || flag("--force-with-lease")
                || short_has('f')
                || rest
                    .iter()
                    .any(|a| a.starts_with('+') || a.starts_with("--force-with-lease="));
            if force {
                danger("git push --force")
            } else {
                Allow
            }
        }
        "commit" if !cfg.allow_git_commit => Deny(deny_commit()),
        "reset" if flag("--hard") => danger("git reset --hard"),
        "clean" if flag("--force") || short_has('f') => danger("git clean -f"),
        "checkout"
            if flag("--") || rest.iter().any(|a| a == ".") || flag("--force") || short_has('f') =>
        {
            danger("git checkout -- <path>")
        }
        "restore" if (!flag("--staged") && !short_has('S')) || flag("--worktree") => {
            danger("git restore")
        }
        "stash" if matches!(rest.first().map(String::as_str), Some("drop" | "clear")) => {
            danger("git stash drop/clear")
        }
        "branch"
            if short_has('D')
                || ((flag("--delete") || short_has('d'))
                    && (flag("--force") || short_has('f'))) =>
        {
            danger("git branch -D")
        }
        _ => Allow,
    }
}

#[path = "policy_release.rs"]
mod policy_release;

#[path = "policy_exec.rs"]
mod policy_exec;
pub use policy_exec::{
    gate_node_exec_current, gate_node_exec_file_current, gate_npm_exec_args_current,
    gate_npm_exec_current,
};

pub use super::policy_scope::{external_roots, prompt_addendum, validate_external_path};

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "policy_release_tests.rs"]
mod release_tests;
