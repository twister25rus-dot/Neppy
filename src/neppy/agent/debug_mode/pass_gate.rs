//! The `pass` gate: a task may only be recorded as `pass` when a real check
//! passed after its last file edit.
//!
//! "Real" means a [`ValidationRecord`] produced by `ops::run_check` (via the
//! `debug_run_check` tool): a non-empty argv and a captured exit code. The
//! entries an agent lists in its own `debug_report` payload are self-attested
//! (empty argv, no exit code) and never satisfy the gate.

use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use super::types::ValidationRecord;

/// Shown to the agent when the gate downgrades a `pass`.
pub(super) const NO_CHECK_AFTER_EDIT: &str =
    "DOWNGRADED from pass to partial: no passing check ran after the last edit (run \
     `debug_run_check` with the relevant tests/typecheck, then call `debug_report` again).";

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
    let ok = validation.iter().any(|v| {
        is_real_check(v)
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
