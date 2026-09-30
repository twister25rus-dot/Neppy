use std::time::Duration;

use crate::neppy::security::policy::AutonomyLevel;

use super::*;

fn policy(level: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: level,
        ..SecurityPolicy::default()
    }
}

#[tokio::test]
async fn a_passing_command_reports_zero_and_its_output() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_test_command(
        dir.path(),
        "echo hello; echo oops 1>&2",
        Duration::from_secs(10),
    )
    .await;
    assert!(result.passed());
    assert!(result.tail.contains("hello") && result.tail.contains("oops"));
}

#[tokio::test]
async fn a_failing_command_reports_its_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_test_command(dir.path(), "exit 7", Duration::from_secs(10)).await;
    assert_eq!(result.exit_code, 7);
    assert!(!result.passed());
}

#[tokio::test]
async fn the_command_runs_in_the_project_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("marker.txt"), "x").unwrap();
    let result = run_test_command(dir.path(), "ls", Duration::from_secs(10)).await;
    assert!(result.tail.contains("marker.txt"));
}

#[tokio::test]
async fn output_is_capped_to_the_last_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let cmd =
        "i=0; while [ $i -lt 4000 ]; do echo line-$i-padding-padding-padding; i=$((i+1)); done";
    let result = run_test_command(dir.path(), cmd, Duration::from_secs(20)).await;
    assert!(result.tail.len() <= TEST_TAIL_RUN);
    assert!(result.tail.contains("line-3999-"));
    assert!(!result.tail.contains("line-0-"));
}

#[tokio::test]
async fn a_timeout_kills_the_whole_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let started = std::time::Instant::now();
    let result = run_test_command(
        dir.path(),
        "sleep 30 & sleep 30",
        Duration::from_millis(300),
    )
    .await;
    assert!(result.timed_out);
    assert!(!result.passed());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "did not wait for the sleeps"
    );
    assert!(result.tail.contains("timed out"));
}

#[test]
fn the_policy_decides_what_may_run() {
    assert!(check_command(&policy(AutonomyLevel::Full), "cargo test").is_ok());
    assert!(check_command(&policy(AutonomyLevel::Supervised), "cargo test").is_ok());
    let read_only = policy(AutonomyLevel::ReadOnly);
    assert!(check_command(&read_only, "cargo test").is_err());
    assert!(check_command(&read_only, "git status").is_ok());
    assert!(check_command(&policy(AutonomyLevel::Full), "rm -rf /").is_err());
}

#[test]
fn a_refused_result_says_why_and_is_not_a_pass() {
    let r = refused_result("blocked");
    assert_eq!(r.exit_code, TEST_REFUSED);
    assert!(!r.passed());
    assert!(r.tail.contains("blocked"));
}
