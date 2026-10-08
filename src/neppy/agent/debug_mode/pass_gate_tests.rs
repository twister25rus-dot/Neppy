use super::*;
use crate::neppy::agent::debug_mode::types::{CheckKind, DebugCheck};

fn rec(command: &[&str], exit: Option<i32>, passed: bool, at: DateTime<Utc>) -> ValidationRecord {
    ValidationRecord {
        check_id: None,
        command: command.iter().map(|s| s.to_string()).collect(),
        exit_code: exit,
        passed,
        timed_out: false,
        duration_ms: 1,
        at: at.to_rfc3339(),
        output_tail: String::new(),
    }
}

fn edited_dir() -> (tempfile::TempDir, DateTime<Utc>) {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("a.txt"), "x").unwrap();
    let mtime = last_edit_time(d.path(), &["a.txt".to_string()]).unwrap();
    (d, mtime)
}

fn files() -> Vec<String> {
    vec!["a.txt".to_string()]
}

#[test]
fn no_changed_files_needs_no_check() {
    let d = tempfile::tempdir().unwrap();
    assert!(missing_check_note(d.path(), &[], &[]).is_none());
}

#[test]
fn a_passing_real_check_after_the_edit_satisfies_the_gate() {
    let (d, edit) = edited_dir();
    let v = [rec(
        &["cargo", "test"],
        Some(0),
        true,
        edit + chrono::Duration::seconds(1),
    )];
    assert!(missing_check_note(d.path(), &files(), &v).is_none());
}

#[test]
fn a_check_older_than_the_last_edit_does_not_count() {
    let (d, edit) = edited_dir();
    let v = [rec(
        &["cargo", "test"],
        Some(0),
        true,
        edit - chrono::Duration::seconds(1),
    )];
    let note = missing_check_note(d.path(), &files(), &v).unwrap();
    assert!(
        note.starts_with("DOWNGRADED from pass to partial"),
        "{note}"
    );
    assert!(note.contains("debug_run_check"));
}

#[test]
fn self_attested_failed_and_timed_out_records_never_count() {
    let (d, edit) = edited_dir();
    let later = edit + chrono::Duration::seconds(5);
    let mut timed_out = rec(&["cargo", "test"], Some(0), true, later);
    timed_out.timed_out = true;
    let v = [
        // Self-attested: empty argv, no exit code.
        rec(&[], None, true, later),
        // Argv but no exit code.
        rec(&["cargo", "test"], None, true, later),
        // Exit code but no argv.
        rec(&[], Some(0), true, later),
        // A real run that failed.
        rec(&["cargo", "test"], Some(1), false, later),
        timed_out,
    ];
    assert!(missing_check_note(d.path(), &files(), &v).is_some());
    assert!(missing_check_note(d.path(), &files(), &[]).is_some());
}

#[test]
fn unparseable_timestamps_do_not_count() {
    let (d, _) = edited_dir();
    let mut r = rec(&["cargo", "test"], Some(0), true, Utc::now());
    r.at = "yesterday".into();
    assert!(missing_check_note(d.path(), &files(), &[r]).is_some());
}

#[test]
fn deleted_files_are_skipped_when_dating_the_last_edit() {
    let (d, edit) = edited_dir();
    let both = vec!["a.txt".to_string(), "gone.txt".to_string()];
    assert_eq!(last_edit_time(d.path(), &both), Some(edit));
    assert_eq!(last_edit_time(d.path(), &["gone.txt".to_string()]), None);
    // Only deleted files changed: any real passing run satisfies the gate.
    let v = [rec(&["cargo", "test"], Some(0), true, Utc::now())];
    assert!(missing_check_note(d.path(), &["gone.txt".to_string()], &v).is_none());
    assert!(missing_check_note(d.path(), &["gone.txt".to_string()], &[]).is_some());
}

#[test]
fn last_edit_is_the_newest_mtime_of_the_changed_files() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("old.txt"), "x").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(30));
    std::fs::write(d.path().join("new.txt"), "x").unwrap();
    let old = last_edit_time(d.path(), &["old.txt".to_string()]).unwrap();
    let both = last_edit_time(d.path(), &["old.txt".to_string(), "new.txt".to_string()]).unwrap();
    assert!(both > old);
}

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

#[test]
fn classify_argv_table() {
    let counts = [
        "cargo test",
        "cargo test --lib -- debug_mode",
        "cargo check",
        "cargo check --all-targets",
        "cargo clippy -- -D warnings",
        "cargo build --release",
        "pnpm run test",
        "pnpm run test:rust",
        "pnpm run lint",
        "pnpm run typecheck",
        "pnpm run type-check",
        "pnpm run compile",
        "pnpm run build",
        "pnpm run check",
        "npm run test",
        "npm run-script lint",
        "npm test",
        "yarn test",
        "yarn run build",
        "pnpm test",
        "pnpm typecheck",
        "pnpm lint",
        "pnpm build",
        "vitest run",
        "tsc --noEmit",
        "eslint src",
    ];
    for c in counts {
        assert!(classify_argv(&argv(c)), "{c} should count");
    }
    let does_not = [
        "",
        "git status",
        "git diff",
        "git log",
        "git show HEAD",
        "git rev-parse HEAD",
        "git ls-files",
        "git ls-tree HEAD",
        "git describe",
        "git blame a.txt",
        "git shortlog",
        "cargo tree",
        "cargo metadata",
        "cargo fmt --check",
        "cargo",
        "npm run format",
        "npm run format:check",
        "pnpm run check:format",
        "pnpm run prettier:check",
        "pnpm run dev",
        "pnpm run start",
        "pnpm run debug:check",
        "npm install",
        "npm",
        "pnpm format",
        "node -e 1",
        "echo test",
        "make test",
    ];
    for c in does_not {
        assert!(!classify_argv(&argv(c)), "{c} should not count");
    }
}

#[test]
fn discovered_checks_decide_by_kind() {
    let chk = |cmd: &str, kind| DebugCheck {
        id: cmd.into(),
        label: cmd.into(),
        command: argv(cmd),
        kind,
        preferred: false,
    };
    let discovered = [
        chk("pnpm run debug:check", CheckKind::Test),
        chk("pnpm run tsc", CheckKind::Typecheck),
        chk("pnpm run format:check", CheckKind::Format),
        chk("cargo fmt --check", CheckKind::Format),
    ];
    // The project's own aggregate script counts although its name is unknown.
    assert!(is_verification(&argv("pnpm run debug:check"), &discovered));
    assert!(is_verification(&argv("pnpm run tsc"), &discovered));
    assert!(!is_verification(
        &argv("pnpm run format:check"),
        &discovered
    ));
    assert!(!is_verification(&argv("cargo fmt --check"), &discovered));
    // Not discovered: falls back to the argv classifier.
    assert!(is_verification(&argv("cargo test"), &discovered));
    assert!(!is_verification(&argv("git rev-parse HEAD"), &discovered));
}

#[test]
fn a_real_passing_non_verification_command_does_not_satisfy_the_gate() {
    let (d, edit) = edited_dir();
    let later = edit + chrono::Duration::seconds(1);
    for cmd in [
        &["git", "rev-parse", "HEAD"][..],
        &["git", "status"],
        &["cargo", "tree"],
        &["cargo", "metadata"],
        &["cargo", "fmt", "--check"],
    ] {
        let v = [rec(cmd, Some(0), true, later)];
        let note = missing_check_note(d.path(), &files(), &v).unwrap();
        assert!(note.contains("TEST, TYPECHECK, LINT or BUILD"), "{note}");
    }
    let v = [rec(&["cargo", "test"], Some(0), true, later)];
    assert!(missing_check_note(d.path(), &files(), &v).is_none());
}
