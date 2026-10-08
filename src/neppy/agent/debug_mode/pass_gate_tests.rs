use super::*;

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
