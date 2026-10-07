use super::*;

const NOW: &str = "2026-10-07T00:00:00Z";
const DEAD: &dyn Fn(u32) -> bool = &|_| false;
const ALIVE: &dyn Fn(u32) -> bool = &|_| true;

#[test]
fn first_launch_writes_a_marker_and_reports_nothing() {
    let d = tempfile::tempdir().unwrap();
    assert!(begin_in(d.path(), 10, "1.0.0", NOW, DEAD).is_none());
    let m: LaunchMarker =
        serde_json::from_slice(&fs::read(d.path().join(PENDING_FILE)).unwrap()).unwrap();
    assert_eq!(m.pid, 10);
    assert_eq!(m.version, "1.0.0");
    assert!(!d.path().join(FAILED_FILE).exists());
}

#[test]
fn a_clean_launch_leaves_no_marker_for_the_next_one() {
    let d = tempfile::tempdir().unwrap();
    begin_in(d.path(), 10, "1.0.0", NOW, DEAD);
    assert!(mark_ready_in(d.path()));
    assert!(!mark_ready_in(d.path()), "second clear is a no-op");
    assert!(begin_in(d.path(), 11, "1.0.0", NOW, DEAD).is_none());
    assert!(!d.path().join(FAILED_FILE).exists());
}

#[test]
fn a_stale_marker_from_a_dead_process_is_a_failed_launch() {
    let d = tempfile::tempdir().unwrap();
    begin_in(d.path(), 10, "0.9.0", "2026-10-06T00:00:00Z", DEAD);
    let failed = begin_in(d.path(), 11, "1.0.0", NOW, DEAD).expect("detected");
    assert_eq!(failed.pid, 10);
    assert_eq!(failed.version, "0.9.0");
    assert_eq!(failed.started_at, "2026-10-06T00:00:00Z");
    assert_eq!(failed.detected_at, NOW);
    let saved: FailedLaunch =
        serde_json::from_slice(&fs::read(d.path().join(FAILED_FILE)).unwrap()).unwrap();
    assert_eq!(saved, failed);
    // The new launch owns the pending marker now.
    let m: LaunchMarker =
        serde_json::from_slice(&fs::read(d.path().join(PENDING_FILE)).unwrap()).unwrap();
    assert_eq!(m.pid, 11);
}

#[test]
fn a_marker_from_a_live_other_instance_is_not_a_failure() {
    let d = tempfile::tempdir().unwrap();
    begin_in(d.path(), 10, "1.0.0", NOW, DEAD);
    assert!(begin_in(d.path(), 11, "1.0.0", NOW, ALIVE).is_none());
    assert!(!d.path().join(FAILED_FILE).exists());
}

#[test]
fn a_marker_from_this_very_pid_counts_as_failed() {
    let d = tempfile::tempdir().unwrap();
    begin_in(d.path(), 10, "1.0.0", NOW, DEAD);
    // Same pid again (restart in place) cannot be a concurrent instance.
    assert!(begin_in(d.path(), 10, "1.0.0", NOW, ALIVE).is_some());
}

#[test]
fn an_unreadable_marker_is_treated_as_failed_not_ignored() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join(PENDING_FILE), b"{ not json").unwrap();
    let failed = begin_in(d.path(), 11, "1.0.0", NOW, ALIVE).expect("detected");
    assert_eq!(failed.pid, 0);
    assert_eq!(failed.version, "unknown");
}

#[test]
fn begin_creates_the_data_dir_and_survives_a_write_failure() {
    let d = tempfile::tempdir().unwrap();
    let nested = d.path().join("a").join("b");
    assert!(begin_in(&nested, 1, "v", NOW, DEAD).is_none());
    assert!(nested.join(PENDING_FILE).is_file());
    // A path under a regular file cannot be created: logged, no panic.
    let file = d.path().join("plain");
    fs::write(&file, b"x").unwrap();
    assert!(begin_in(&file.join("sub"), 1, "v", NOW, DEAD).is_none());
}

#[cfg(unix)]
#[test]
fn process_alive_sees_this_process_but_not_a_reaped_one() {
    assert!(process_alive(std::process::id()));
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    assert!(!process_alive(pid));
}
