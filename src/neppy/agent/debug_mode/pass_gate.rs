//! The `pass` gate: a task may only be recorded as `pass` when a real
//! VERIFICATION check (test, typecheck, lint or build) passed after its last
//! file edit.
//!
//! "Real" means a [`ValidationRecord`] produced by `ops::run_check` (via the
//! `debug_run_check` tool): a non-empty argv and a captured exit code. The
//! entries an agent lists in its own `debug_report` payload are self-attested
//! (empty argv, no exit code) and never satisfy the gate. A real run of a
//! command that verifies nothing (`git rev-parse HEAD`, `cargo tree`, ...) does
//! not satisfy it either: see [`is_verification`].

use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use super::checks;
use super::types::{CheckKind, DebugCheck, ValidationRecord};

/// Shown to the agent when the gate downgrades a `pass`.
pub(super) const NO_CHECK_AFTER_EDIT: &str =
    "DOWNGRADED from pass to partial: no passing TEST, TYPECHECK, LINT or BUILD check ran \
     after the last edit (run `debug_run_check` with the relevant tests/typecheck/lint/build, \
     then call `debug_report` again). Commands that verify nothing, such as git status/diff/\
     rev-parse or cargo tree/metadata/fmt, do not count.";

/// Script-name prefixes that mark a package.json script as a verification run.
const VERIFY_SCRIPT_PREFIXES: &[&str] = &[
    "test",
    "typecheck",
    "type-check",
    "compile",
    "lint",
    "build",
    "check",
];
/// Names that look like a verification prefix but only check/rewrite formatting.
const NOT_VERIFY_SCRIPT_MARKERS: &[&str] = &["format", "fmt", "prettier"];
/// cargo subcommands that verify the code.
const CARGO_VERIFY: &[&str] = &["test", "check", "clippy", "build"];
/// Tools that verify the code when invoked directly.
const DIRECT_VERIFY_TOOLS: &[&str] = &["vitest", "tsc", "eslint"];

/// Flags that make a command print information instead of running anything.
/// Anywhere in the argv (including after a `--` passthrough) they disqualify
/// it: `cargo build --help` and `npm run test -- --version` verify nothing.
const INFO_ONLY_FLAGS: &[&str] = &["--help", "-h", "--version", "-V", "--list"];

fn is_verify_script(name: &str) -> bool {
    !NOT_VERIFY_SCRIPT_MARKERS.iter().any(|m| name.contains(m))
        && VERIFY_SCRIPT_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Pure, conservative classification of an argv as a verification command
/// (test, typecheck, lint or build). Unknown commands do not count.
///
/// * `cargo test|check|clippy|build ...` counts; `cargo tree|metadata|fmt` do not;
/// * `git ...` never counts;
/// * `npm|pnpm|yarn run <script>` (and bare `test`, plus bare `pnpm`/`yarn
///   <script>`) count when the script name is or starts with
///   test / typecheck / type-check / compile / lint / build / check, unless it
///   is a formatting script;
/// * direct `vitest`, `tsc`, `eslint` count;
/// * any argv carrying an info-only flag (`--help`, `-h`, `--version`, `-V`,
///   `--list`) never counts, wherever the flag appears.
pub(super) fn classify_argv(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };
    if argv
        .iter()
        .skip(1)
        .any(|a| INFO_ONLY_FLAGS.contains(&a.as_str()))
    {
        return false;
    }
    let base = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let sub = argv.get(1).map(String::as_str);
    match base {
        "cargo" => sub.is_some_and(|s| CARGO_VERIFY.contains(&s)),
        "npm" | "pnpm" | "yarn" => match sub {
            Some("run" | "run-script") => argv.get(2).is_some_and(|s| is_verify_script(s)),
            Some("test") => true,
            Some(s) if base != "npm" => is_verify_script(s),
            _ => false,
        },
        b if DIRECT_VERIFY_TOOLS.contains(&b) => true,
        _ => false,
    }
}

/// Whether running `argv` verifies the code. A discovered check with exactly
/// this argv decides by its kind (test / typecheck / lint / build count;
/// format does not); anything else falls back to [`classify_argv`].
pub(super) fn is_verification(argv: &[String], discovered: &[DebugCheck]) -> bool {
    match discovered.iter().find(|c| c.command == argv) {
        Some(c) => matches!(
            c.kind,
            CheckKind::Test | CheckKind::Typecheck | CheckKind::Lint | CheckKind::Build
        ),
        None => classify_argv(argv),
    }
}

/// True for a record that came from an actual command run (not self-attested).
pub(super) fn is_real_check(v: &ValidationRecord) -> bool {
    !v.command.is_empty() && v.exit_code.is_some()
}

/// Newest filesystem mtime among `files` (relative to `root`). Files that no
/// longer exist (deleted by the task) or cannot be stat'ed are skipped.
pub(super) fn last_edit_time(root: &Path, files: &[String]) -> Option<DateTime<Utc>> {
    files
        .iter()
        .filter_map(|f| {
            let m = std::fs::symlink_metadata(root.join(f)).ok()?;
            m.modified().ok()
        })
        .max()
        .map(|t: SystemTime| DateTime::<Utc>::from(t))
}

fn parse_at(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

/// `None` when `pass` may stand; otherwise the downgrade message.
///
/// * no changed files: no check is needed;
/// * otherwise a real, passing, non-timed-out record must exist, stamped later
///   than the last edit (any real passing record when no edited file still
///   exists to date the edit).
pub(super) fn missing_check_note(
    root: &Path,
    files: &[String],
    validation: &[ValidationRecord],
) -> Option<String> {
    if files.is_empty() {
        log::debug!("[debug_mode] pass gate: no changed files, no check needed");
        return None;
    }
    let last_edit = last_edit_time(root, files);
    let discovered = checks::discover(root);
    let ok = validation.iter().any(|v| {
        is_real_check(v)
            && is_verification(&v.command, &discovered)
            && v.passed
            && !v.timed_out
            && match (last_edit, parse_at(&v.at)) {
                (Some(edit), Some(at)) => at > edit,
                (None, Some(_)) => true,
                (_, None) => false,
            }
    });
    log::debug!(
        "[debug_mode] pass gate: files={} records={} real={} last_edit={} satisfied={ok}",
        files.len(),
        validation.len(),
        validation.iter().filter(|v| is_real_check(v)).count(),
        last_edit.map_or_else(|| "-".to_string(), |t| t.to_rfc3339())
    );
    (!ok).then(|| NO_CHECK_AFTER_EDIT.to_string())
}

#[cfg(test)]
#[path = "pass_gate_tests.rs"]
mod tests;
