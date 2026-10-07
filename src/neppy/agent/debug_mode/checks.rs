//! Validation-check discovery and the `run_check` command policy.
//!
//! Checks are derived from the project's own files (`package.json` scripts,
//! `Cargo.toml`, the lockfile that picks the package manager) — never from a
//! hardcoded list of Neppy's commands.

use std::path::Path;

use super::types::{CheckKind, DebugCheck};

/// Programs `run_check` will execute even when the argv was not discovered.
pub const ALLOWED_PROGRAMS: &[&str] = &["npm", "pnpm", "yarn", "cargo", "git"];

/// `package.json` script names recognised as validation checks. Mutating
/// scripts (plain `format`) are deliberately absent.
const SCRIPT_KINDS: &[(&str, CheckKind)] = &[
    ("lint", CheckKind::Lint),
    ("typecheck", CheckKind::Typecheck),
    ("type-check", CheckKind::Typecheck),
    ("tsc", CheckKind::Typecheck),
    ("compile", CheckKind::Typecheck),
    ("test", CheckKind::Test),
    ("test:unit", CheckKind::Test),
    ("build", CheckKind::Build),
    ("format:check", CheckKind::Format),
    ("check:format", CheckKind::Format),
    ("prettier:check", CheckKind::Format),
];

/// Read-only git subcommands `run_check` accepts.
const GIT_READONLY: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "rev-parse",
    "ls-files",
    "ls-tree",
    "describe",
    "blame",
    "shortlog",
];

/// Long git options that run helpers, write files or escape the repo. Matched
/// by prefix in both directions so abbreviations (`--out`) cannot slip through.
const GIT_DENIED_LONG: &[&str] = &[
    "--no-index",
    "--output",
    "--open-files-in-pager",
    "--exec",
    "--ext-diff",
    "--upload-pack",
];

/// cargo subcommands `run_check` accepts (`fmt` additionally needs `--check`).
const CARGO_ALLOWED: &[&str] = &[
    "check", "test", "build", "clippy", "fmt", "tree", "metadata",
];

/// cargo arguments that inject config or point outside the project.
const CARGO_DENIED_ARGS: &[&str] = &[
    "--config",
    "-Z",
    "--manifest-path",
    "--fix",
    "--target-dir",
    "--out-dir",
    "--artifact-dir",
];

/// JS package-manager subcommands that run a project script by name.
const JS_RUN: &[&str] = &["run", "run-script"];
/// pnpm runs a script by bare name; only these, and only if the script exists.
const PNPM_BARE: &[&str] = &["typecheck", "lint", "build"];

fn package_manager(root: &Path, pkg: &serde_json::Value) -> &'static str {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else if root.join("package-lock.json").is_file() {
        "npm"
    } else {
        match pkg.get("packageManager").and_then(|v| v.as_str()) {
            Some(s) if s.starts_with("pnpm") => "pnpm",
            Some(s) if s.starts_with("yarn") => "yarn",
            _ => "npm",
        }
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// Discover validation checks for the project at `root`.
pub fn discover(root: &Path) -> Vec<DebugCheck> {
    let mut checks: Vec<DebugCheck> = Vec::new();

    let pkg_path = root.join("package.json");
    if pkg_path.is_file() {
        match std::fs::read_to_string(&pkg_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        {
            Some(pkg) => {
                let pm = package_manager(root, &pkg);
                let scripts = pkg.get("scripts").and_then(|v| v.as_object());
                let has = |name: &str| -> bool {
                    scripts
                        .and_then(|s| s.get(name))
                        .and_then(|v| v.as_str())
                        // npm's `init` placeholder is not a real test script.
                        .is_some_and(|body| !body.contains("no test specified"))
                };
                if has("debug:check") {
                    checks.push(DebugCheck {
                        id: "debug-check".into(),
                        label: "debug:check".into(),
                        command: argv(&[pm, "run", "debug:check"]),
                        kind: CheckKind::Test,
                        preferred: true,
                    });
                }
                for (name, kind) in SCRIPT_KINDS {
                    if has(name) {
                        checks.push(DebugCheck {
                            id: format!("{pm}:{name}"),
                            label: format!("{pm} run {name}"),
                            command: argv(&[pm, "run", name]),
                            kind: *kind,
                            preferred: false,
                        });
                    }
                }
            }
            None => log::debug!("[debug_mode] package.json unreadable or invalid; skipping"),
        }
    }

    if root.join("Cargo.toml").is_file() {
        for (id, label, cmd, kind) in [
            (
                "cargo:check",
                "cargo check",
                &["cargo", "check"][..],
                CheckKind::Typecheck,
            ),
            (
                "cargo:clippy",
                "cargo clippy",
                &["cargo", "clippy"][..],
                CheckKind::Lint,
            ),
            (
                "cargo:test",
                "cargo test",
                &["cargo", "test"][..],
                CheckKind::Test,
            ),
            (
                "cargo:fmt",
                "cargo fmt --check",
                &["cargo", "fmt", "--check"][..],
                CheckKind::Format,
            ),
        ] {
            checks.push(DebugCheck {
                id: id.into(),
                label: label.into(),
                command: argv(cmd),
                kind,
                preferred: false,
            });
        }
    }
    log::debug!("[debug_mode] discovered {} checks", checks.len());
    checks
}

/// Script names from the project's `package.json`.
fn project_scripts(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("scripts").and_then(|s| s.as_object()).cloned())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// Anything after the script/subcommand must be a `--`-separated pass-through,
/// so no flag can be parsed by the package manager itself.
fn only_passthrough(rest: &[String]) -> Result<(), String> {
    match rest.first() {
        None => Ok(()),
        Some(first) if first == "--" => Ok(()),
        Some(first) => Err(format!(
            "unexpected argument '{first}' (script arguments must follow '--')"
        )),
    }
}

fn validate_js(program: &str, argv: &[String], scripts: &[String]) -> Result<(), String> {
    let sub = argv[1].as_str();
    let has = |name: &str| scripts.iter().any(|s| s == name);
    if JS_RUN.contains(&sub) {
        let script = argv.get(2).map(String::as_str).unwrap_or("");
        if script.starts_with('-') || !has(script) {
            return Err(format!("'{script}' is not a script in package.json"));
        }
        return only_passthrough(&argv[3..]);
    }
    if sub == "test" {
        return only_passthrough(&argv[2..]);
    }
    if program == "pnpm" && PNPM_BARE.contains(&sub) && has(sub) {
        return only_passthrough(&argv[2..]);
    }
    Err(format!("'{program} {sub}' is not allowed via run_check"))
}

fn validate_cargo(argv: &[String]) -> Result<(), String> {
    let sub = argv[1].as_str();
    if !CARGO_ALLOWED.contains(&sub) {
        return Err(format!("'cargo {sub}' is not allowed via run_check"));
    }
    // Everything after a bare `--` belongs to the test binary / tool, not cargo.
    let cargo_args: Vec<&String> = argv[2..].iter().take_while(|a| *a != "--").collect();
    if let Some(bad) = cargo_args.iter().find(|a| {
        a.starts_with("--allow-")
            || CARGO_DENIED_ARGS.iter().any(|d| {
                a.as_str() == *d
                    || a.starts_with(&format!("{d}="))
                    || (*d == "-Z" && a.starts_with("-Z"))
            })
    }) {
        return Err(format!(
            "cargo argument '{bad}' is not allowed via run_check"
        ));
    }
    if sub == "fmt" && !cargo_args.iter().any(|a| a.as_str() == "--check") {
        return Err("'cargo fmt' is only allowed with --check".into());
    }
    Ok(())
}

/// Absolute, home-relative or `..`-containing path-like value.
fn escapes_repo(v: &str) -> bool {
    v.starts_with('/')
        || v.starts_with('~')
        || v.starts_with('\\')
        || v.split(['/', '\\', ':']).any(|c| c == "..")
        || (v.as_bytes().get(1) == Some(&b':') && v.as_bytes()[0].is_ascii_alphabetic())
}

fn validate_git(argv: &[String]) -> Result<(), String> {
    let sub = argv[1].as_str();
    if !GIT_READONLY.contains(&sub) {
        return Err(format!(
            "git subcommand '{sub}' is not allowed via run_check (read-only: {})",
            GIT_READONLY.join(", ")
        ));
    }
    for arg in &argv[2..] {
        let name = arg.split('=').next().unwrap_or("");
        let denied_long = name.len() >= 3
            && name.starts_with("--")
            && GIT_DENIED_LONG
                .iter()
                .any(|d| d.starts_with(name) || name.starts_with(d));
        if denied_long || arg == "-c" || arg.starts_with("-O") || arg.starts_with("-C") {
            return Err(format!("git argument '{arg}' is not allowed via run_check"));
        }
        // Paths must stay inside the repo: an out-of-repo path turns `git diff`
        // into an implicit no-index diff of arbitrary files.
        let value = arg.split_once('=').map(|(_, v)| v);
        for candidate in std::iter::once(arg.as_str()).chain(value) {
            if escapes_repo(candidate) {
                return Err(format!(
                    "git argument '{arg}' points outside the project and is not allowed"
                ));
            }
        }
    }
    Ok(())
}

/// Gate for `run_check`: the argv must be a discovered check verbatim, or an
/// allowlisted program with an allowlisted subcommand in argv[1] (a leading
/// flag is never accepted). JS package managers may only run scripts that exist
/// in the project's `package.json`. The argv is never passed through a shell.
pub fn validate_command(
    root: &Path,
    argv: &[String],
    discovered: &[DebugCheck],
) -> Result<(), String> {
    let Some(program) = argv.first() else {
        return Err("empty command".into());
    };
    if argv.iter().any(|a| a.contains('\0')) {
        return Err("command contains a NUL byte".into());
    }
    if discovered.iter().any(|c| c.command == argv) {
        return Ok(());
    }
    if !ALLOWED_PROGRAMS.contains(&program.as_str()) {
        return Err(format!(
            "'{program}' is not a discovered check and not in the allowlist ({})",
            ALLOWED_PROGRAMS.join(", ")
        ));
    }
    match argv.get(1) {
        None => return Err(format!("'{program}' needs a subcommand")),
        Some(sub) if sub.starts_with('-') => {
            return Err(format!(
                "'{sub}': the subcommand must come first, before any flag"
            ))
        }
        Some(_) => {}
    }
    match program.as_str() {
        "git" => validate_git(argv),
        "cargo" => validate_cargo(argv),
        _ => validate_js(program, argv, &project_scripts(root)),
    }
}
