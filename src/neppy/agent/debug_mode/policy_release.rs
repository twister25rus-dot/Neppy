//! Debug Mode may never publish a release. Publishing pushes commits and a tag
//! and creates a GitHub release signed with the user's key; the user does that
//! from the Debug panel's "Publish release" card (an RPC the agent has no tool
//! for), so the shell is the one remaining path this closes.
//!
//! The rule decides by what a command *runs*, not by what it mentions:
//! executing `release-neppy.sh` (directly, through a shell or `source`, through
//! `env`/`nohup`/`time`/`xargs`/`sudo`/`pnpm exec`/`find -exec`, or inside a
//! `-c` string, which the caller recurses into), any mutating `gh release` or
//! `gh api .../releases` call, and any package script that wraps a release are
//! denied. `cat`, `grep`, `rg`, `gh release list|view` only read, so they pass.
//!
//! Best effort by nature: a copy of the script under another name, or code that
//! builds the command at run time, is beyond a static check. The approval gate
//! and the tier still apply on top.

use std::path::Path;

use super::DebugCommandDecision;

pub(super) const DENY_MESSAGE: &str = "Publishing a release is not allowed from the agent; the user publishes from the Debug panel's Publish release card.";
const SCRIPT: &str = "release-neppy.sh";
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh", "source", "."];

pub(super) fn deny() -> DebugCommandDecision {
    log::info!("[debug_mode][release] shell command denied: it would publish a release");
    DebugCommandDecision::Deny(DENY_MESSAGE.to_string())
}

fn base(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// True when the simple command `w` (wrappers already stripped) would publish.
/// `extra` are package-script names known to wrap a release.
pub(super) fn publishes_release(w: &[String], extra: &[String]) -> bool {
    let Some(first) = w.first() else { return false };
    let prog = base(first);
    let args = &w[1..];
    if prog == SCRIPT {
        return true;
    }
    match prog {
        p if SHELLS.contains(&p) => {
            // A `-c` string is checked by the caller's recursion; otherwise any
            // operand naming the script is the file being run.
            let has_c = args
                .iter()
                .any(|a| a.starts_with('-') && !a.starts_with("--") && a.ends_with('c'));
            !has_c && args.iter().any(|a| base(a) == SCRIPT)
        }
        "sudo" | "doas" => {
            let rest = skip_flags(args);
            publishes_release(super::strip_wrappers(rest), extra)
        }
        "find" => {
            args.iter()
                .any(|a| matches!(a.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir"))
                && args.iter().any(|a| base(a) == SCRIPT)
        }
        "gh" => gh_publishes(args),
        "npx" | "pnpx" | "bunx" => {
            let rest = skip_flags(args);
            publishes_release(super::strip_wrappers(rest), extra)
        }
        "npm" | "pnpm" | "yarn" | "bun" => js_publishes(args, extra),
        _ => false,
    }
}

fn skip_flags(mut w: &[String]) -> &[String] {
    while w.first().is_some_and(|x| x.starts_with('-')) {
        w = &w[1..];
    }
    w
}

/// Everything except `gh release list|view`, and `gh api` calls that write to
/// a releases endpoint.
fn gh_publishes(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        i += if matches!(args[i].as_str(), "-R" | "--repo" | "--hostname") {
            2
        } else {
            1
        };
    }
    let rest = args.get(i + 1..).unwrap_or(&[]);
    match args.get(i).map(String::as_str) {
        Some("release") => !matches!(
            rest.iter()
                .find(|a| !a.starts_with('-'))
                .map(String::as_str),
            Some("list" | "view")
        ),
        Some("api") => {
            let mentions = rest
                .iter()
                .any(|a| a.to_ascii_lowercase().contains("release"));
            let writes = rest.iter().enumerate().any(|(n, a)| match a.as_str() {
                "-f" | "-F" | "--field" | "--raw-field" | "--input" => true,
                "-X" | "--method" => rest
                    .get(n + 1)
                    .is_none_or(|m| !m.eq_ignore_ascii_case("GET")),
                _ => false,
            });
            mentions && writes
        }
        _ => false,
    }
}

/// A package script invocation: `pnpm release`, `npm run release:x`,
/// `yarn release`, `pnpm exec bash scripts/release-neppy.sh`, ...
fn js_publishes(args: &[String], extra: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() && args[i].starts_with('-') {
        let takes = matches!(
            args[i].as_str(),
            "--filter" | "-F" | "-C" | "--dir" | "--prefix" | "--cwd" | "-w" | "--workspace"
        );
        i += if takes { 2 } else { 1 };
    }
    let Some(sub) = args.get(i).map(String::as_str) else {
        return false;
    };
    let rest = &args[i + 1..];
    match sub {
        "run" | "run-script" | "rum" | "urun" => rest
            .iter()
            .find(|a| !a.starts_with('-'))
            .is_some_and(|s| is_release_script(s, extra)),
        "exec" | "dlx" | "x" => publishes_release(super::strip_wrappers(skip_flags(rest)), extra),
        other => is_release_script(other, extra),
    }
}

fn is_release_script(name: &str, extra: &[String]) -> bool {
    name.to_ascii_lowercase().starts_with("release") || extra.iter().any(|e| e == name)
}

/// Names of scripts in `<root>/package.json` and `<root>/app/package.json` that
/// wrap a release under a name that does not say so (`release*` names are
/// always denied without looking).
pub(super) fn wrapping_scripts(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for rel in ["package.json", "app/package.json"] {
        let Ok(bytes) = std::fs::read(root.join(rel)) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) else {
            continue;
        };
        for (name, body) in scripts {
            let body = body.as_str().unwrap_or("").to_ascii_lowercase();
            let wraps = body.contains("release-neppy")
                || body.contains("gh release")
                || ["run release", "pnpm release", "yarn release", "bun release"]
                    .iter()
                    .any(|p| body.contains(p));
            if wraps {
                out.push(name.clone());
            }
        }
    }
    log::debug!(
        "[debug_mode][release] package scripts that wrap a release: {}",
        out.len()
    );
    out
}

/// A `-c`/`eval` string nested too deep to analyse still must not smuggle the script.
pub(super) fn mentions_script(text: &str) -> bool {
    text.contains(SCRIPT)
}
