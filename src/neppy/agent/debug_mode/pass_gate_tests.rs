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
        // Info-only invocations run nothing, wherever the flag sits.
        "cargo build --help",
        "cargo check -h",
        "cargo test --help",
        "cargo test -- --list",
        "cargo test --list",
        "cargo clippy --version",
        "cargo build -V",
        "npm run test -- --version",
        "npm run test -- --help",
        "pnpm run lint --help",
        "pnpm test -h",
        "yarn run build --list",
        "vitest --help",
        "vitest run --version",
        "tsc --version",
        "tsc -h",
        "eslint --help",
        "eslint -V",
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

// ── zero-test runs ──────────────────────────────────────────────────────

const CARGO_ZERO_LIB: &str = "\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 612 filtered out; finished in 0.00s\n\n   Doc-tests neppy\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
const CARGO_RAN: &str = "\nrunning 12 tests\ntest a::b ... ok\ntest a::c ... ok\n\ntest result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 600 filtered out; finished in 0.31s\n\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
const CARGO_ONE_TEST: &str = "running 1 test\ntest x ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
const VITEST_NO_FILES: &str =
    "\n No test files found, exiting with code 0\n\nfilter: src/nope.test.ts\n";
const VITEST_NO_FILES_ANSI: &str = "\u{1b}[31mNo test files found\u{1b}[39m, exiting with code 0\n";
const VITEST_ZERO: &str =
    " Test Files  1 passed (1)\n      Tests  0 passed (0)\n   Duration  1.2s\n";
const VITEST_RAN: &str =
    " Test Files  2 passed (2)\n      Tests  14 passed (14)\n   Duration  3.1s\n";
const VITEST_SOME_FAILED: &str =
    " Test Files  1 failed (1)\n      Tests  1 failed | 0 passed (1)\n";
const JEST_ZERO: &str = "Tests:       0 total\nTest Suites: 0 total\n";
const JEST_RAN: &str = "Tests:       3 passed, 3 total\nTest Suites: 1 passed, 1 total\n";

#[test]
fn ran_no_tests_table() {
    for (name, out, want) in [
        ("cargo, filter matched nothing", CARGO_ZERO_LIB, true),
        ("cargo, a section ran 12", CARGO_RAN, false),
        ("cargo, one test", CARGO_ONE_TEST, false),
        ("vitest no files", VITEST_NO_FILES, true),
        (
            "vitest no files with colour codes",
            VITEST_NO_FILES_ANSI,
            true,
        ),
        ("vitest 0 passed", VITEST_ZERO, true),
        ("vitest ran", VITEST_RAN, false),
        ("vitest failures counted as ran", VITEST_SOME_FAILED, false),
        ("jest 0 total", JEST_ZERO, true),
        ("jest ran", JEST_RAN, false),
        ("empty output is inconclusive", "", false),
        (
            "unrelated output is inconclusive",
            "compiling...\nfinished\n",
            false,
        ),
        (
            "a test name that says no tests is not the message",
            "test x::handles_no_tests_yet ... ok\ntest result: ok. 1 passed\n",
            false,
        ),
    ] {
        assert_eq!(ran_no_tests(out), want, "{name}");
    }
}

#[test]
fn a_clipped_tail_is_inconclusive() {
    // The stored tail is the last 4096 chars: the sections that ran tests may
    // be gone, so only the zero-looking end is visible. That must not downgrade.
    let clipped = format!("{}{CARGO_ZERO_LIB}", "x".repeat(STORED_OUTPUT_TAIL));
    assert!(!ran_no_tests(&clipped));
}

#[test]
fn is_test_run_table() {
    for (cmd, want) in [
        ("cargo test --lib", true),
        ("cargo nextest run", true),
        ("npm run test", true),
        ("pnpm run test:rust", true),
        ("pnpm test", true),
        ("yarn test", true),
        ("vitest run", true),
        ("jest", true),
        ("cargo check", false),
        ("cargo clippy", false),
        ("pnpm run typecheck", false),
        ("pnpm run lint", false),
        ("pnpm run build", false),
        ("tsc --noEmit", false),
        ("", false),
    ] {
        assert_eq!(is_test_run(&argv(cmd), &[]), want, "{cmd}");
    }
}

#[test]
fn a_discovered_check_decides_by_its_kind() {
    let lint = DebugCheck {
        id: "x".into(),
        label: "x".into(),
        command: argv("pnpm run test-lint"),
        kind: CheckKind::Lint,
        preferred: false,
    };
    assert!(!is_test_run(&argv("pnpm run test-lint"), &[lint]));
}

fn record_with_output(command: &[&str], out: &str, at: DateTime<Utc>) -> ValidationRecord {
    let mut r = rec(command, Some(0), true, at);
    r.output_tail = out.to_string();
    r
}

#[test]
fn a_zero_test_run_does_not_satisfy_the_gate_but_a_normal_run_does() {
    let (d, edit) = edited_dir();
    let later = edit + chrono::Duration::seconds(1);
    for (cmd, zero, ran) in [
        (
            &["cargo", "test", "--lib", "nope"][..],
            CARGO_ZERO_LIB,
            CARGO_RAN,
        ),
        (
            &["vitest", "run", "src/nope"][..],
            VITEST_NO_FILES,
            VITEST_RAN,
        ),
        (&["vitest", "run"][..], VITEST_ZERO, VITEST_RAN),
        (&["pnpm", "test"][..], JEST_ZERO, JEST_RAN),
    ] {
        let z = [record_with_output(cmd, zero, later)];
        let note = missing_check_note(d.path(), &files(), &z)
            .unwrap_or_else(|| panic!("{cmd:?}: a zero-test run must not count"));
        assert!(
            note.starts_with("DOWNGRADED from pass to partial"),
            "{note}"
        );
        assert!(note.contains("zero tests"), "{note}");
        let r = [record_with_output(cmd, ran, later)];
        assert!(
            missing_check_note(d.path(), &files(), &r).is_none(),
            "{cmd:?}: a normal run still counts"
        );
        // A zero-test run next to a real verification still passes.
        let both = [
            record_with_output(cmd, zero, later),
            record_with_output(&["tsc", "--noEmit"], "", later),
        ];
        assert!(
            missing_check_note(d.path(), &files(), &both).is_none(),
            "{cmd:?}"
        );
    }
}

#[test]
fn typecheck_lint_and_build_records_are_not_affected_by_zero_test_output() {
    let (d, edit) = edited_dir();
    let later = edit + chrono::Duration::seconds(1);
    for cmd in [
        &["cargo", "check"][..],
        &["cargo", "clippy"][..],
        &["cargo", "build"][..],
        &["tsc", "--noEmit"][..],
        &["eslint", "src"][..],
        &["pnpm", "run", "typecheck"][..],
        &["pnpm", "run", "build"][..],
    ] {
        // Even output that happens to say "running 0 tests".
        let v = [record_with_output(cmd, CARGO_ZERO_LIB, later)];
        assert!(
            missing_check_note(d.path(), &files(), &v).is_none(),
            "{cmd:?}"
        );
    }
}
