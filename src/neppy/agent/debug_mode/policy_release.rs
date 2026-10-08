//! Debug Mode may never publish a release. Publishing pushes commits and a tag
//! and creates a GitHub release signed with the user's key; the user does that
//! from the Debug panel's "Publish release" card (an RPC the agent has no tool
//! for), so the shell is the one remaining path this closes.
//!
//! The rule decides by what a command *runs*, not by what it mentions:
//! executing `release-neppy.sh` (directly, through a shell or `source`, through
//! `env`/`nohup`/`time`/`xargs`/`sudo`/`pnpm exec`/`find -exec`, or inside a
//! `-c` string, which the caller recurses into), any mutating `gh release` or
//! `gh api .../releases` call, `gh workflow run|enable` for a release or promote
//! workflow, a `gh api` POST to a `.../dispatches` endpoint, and any package
//! script that wraps a release, and `gh run rerun` (a past run's workflow cannot
//! be resolved from its id) are denied. `cat`, `grep`, `rg`,
//! `gh release list|view`, `gh workflow list|view` and `gh run list|view` only
//! read, so they pass.
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

/// `gh` options that take a value in the next word, wherever they sit.
const GH_VALUE_FLAGS: &[&str] = &[
    "-R",
    "--repo",
    "--hostname",
    "-r",
    "--ref",
    "-f",
    "--raw-field",
    "-F",
    "--field",
    "-X",
    "--method",
    "--input",
    "-H",
    "--header",
    "-q",
    "--jq",
    "-t",
    "--template",
    "--cache",
    "-p",
    "--preview",
];

/// The non-flag words of `args`, skipping the value of each option that takes
/// one (`-R a/b`, `--ref main`, `-X POST`). Attached forms (`--repo=a/b`) are a
/// single flag word and need no skipping.
fn positionals(args: &[String]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
        } else if a.starts_with('-') {
            skip = GH_VALUE_FLAGS.contains(&a.as_str());
        } else {
            out.push(a.as_str());
        }
    }
    out
}

/// Everything except `gh release list|view`; `gh workflow run|enable` for a
/// release or promote workflow; `gh run rerun`; and `gh api` writes to a
/// releases, dispatch, rerun or release-workflow endpoint.
pub(super) fn gh_publishes(args: &[String]) -> bool {
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
        Some("release") => !matches!(positionals(rest).first(), Some(&("list" | "view"))),
        Some("workflow") => workflow_publishes(rest),
        Some("run") => positionals(rest).first() == Some(&"rerun"),
        // Decided by the endpoint path, never by words in field values.
        Some("api") => {
            (api_path_publishes(rest) && gh_api_writes(rest)) || graphql_release_mutation(rest)
        }
        _ => false,
    }
}

/// `gh api graphql` carrying a `mutation` that mentions a release. A plain
/// graphql query, or a mutation that never says release, is not a publish.
fn graphql_release_mutation(rest: &[String]) -> bool {
    if !positionals(rest)
        .iter()
        .any(|p| p.eq_ignore_ascii_case("graphql"))
    {
        return false;
    }
    let text = rest.join(" ").to_ascii_lowercase();
    text.contains("mutation") && text.contains("release")
}

/// True when an operand of `gh api` is a path (or URL) that publishes or
/// triggers a run: a `releases`, `dispatches`, `rerun` or `rerun-failed-jobs`
/// segment (covers `.../runs/<id>/rerun` and `.../jobs/<id>/rerun`), or a
/// release / promote workflow under `actions/workflows`.
fn api_path_publishes(rest: &[String]) -> bool {
    positionals(rest).iter().any(|p| {
        let lower = p.to_ascii_lowercase();
        let path = lower.split(['?', '#']).next().unwrap_or("");
        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        segs.iter().any(|s| {
            matches!(
                *s,
                "releases" | "dispatches" | "rerun" | "rerun-failed-jobs"
            )
        }) || (segs.contains(&"workflows")
            && segs
                .iter()
                .any(|s| s.contains("release") || s.contains("promote")))
    })
}

/// True when the `gh api` operands ask for a mutating request: fields or an
/// input file (gh then defaults to POST) or an explicit non-GET method, in the
/// separated (`-X POST`, `--field k=v`) and the attached (`-XPOST`,
/// `--method=POST`, `-fk=v`, `--field=k=v`) spellings alike.
fn gh_api_writes(rest: &[String]) -> bool {
    rest.iter().enumerate().any(|(n, a)| match a.as_str() {
        "-f" | "-F" | "--field" | "--raw-field" | "--input" => true,
        "-X" | "--method" => rest
            .get(n + 1)
            .is_none_or(|m| !m.eq_ignore_ascii_case("GET")),
        a => {
            if let Some(m) = a.strip_prefix("--method=") {
                return !m.eq_ignore_ascii_case("GET");
            }
            if let Some(m) = a.strip_prefix("-X") {
                return !m.eq_ignore_ascii_case("GET");
            }
            ["--field=", "--raw-field=", "--input="]
                .iter()
                .any(|p| a.starts_with(p))
                || (!a.starts_with("--") && (a.starts_with("-f") || a.starts_with("-F")))
        }
    })
}

/// `gh workflow run|enable <workflow>` where the workflow's file name or name
/// says release or promote (those are `workflow_dispatch` publish paths), or
/// is a bare numeric id, whose name cannot be checked statically. `list`,
/// `view` and the like only read. `-R/--repo <v>` may sit before or after the
/// subcommand.
fn workflow_publishes(rest: &[String]) -> bool {
    let pos = positionals(rest);
    if !matches!(pos.first(), Some(&("run" | "enable"))) {
        return false;
    }
    pos[1..].iter().any(|a| {
        let lower = a.to_ascii_lowercase();
        lower.contains("release")
            || lower.contains("promote")
            || (!a.is_empty() && a.chars().all(|c| c.is_ascii_digit()))
    })
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
