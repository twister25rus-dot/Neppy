use std::time::Duration;

use super::*;

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
fn a_refused_result_says_why_and_is_not_a_pass() {
    let r = refused_result("blocked");
    assert_eq!(r.exit_code, TEST_REFUSED);
    assert!(!r.passed());
    assert!(r.tail.contains("blocked"));
}

// ---- no process outlives its test run ------------------------------------

/// Whether `pid` is a live, non-zombie process.
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    // A killed child can linger as a zombie until its parent reaps it.
    std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|o| {
            let stat = String::from_utf8_lossy(&o.stdout);
            !stat.trim().is_empty() && !stat.trim().starts_with('Z')
        })
        .unwrap_or(false)
}

async fn gone_within(pid: i32, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if !alive(pid) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    !alive(pid)
}

/// The pid a test command printed as `pid=NNN`.
fn printed_pid(tail: &str) -> i32 {
    tail.lines()
        .find_map(|l| l.strip_prefix("pid="))
        .and_then(|n| n.trim().parse().ok())
        .expect("the command prints its background pid")
}

#[cfg(unix)]
#[tokio::test]
async fn a_background_process_does_not_outlive_a_command_that_exited_normally() {
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let result = run_test_command(
        dir.path(),
        "sleep 300 & echo pid=$!; exit 0",
        Duration::from_secs(30),
    )
    .await;
    assert!(result.passed(), "{result:?}");
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "the run must not wait out the pipe-holding grandchild ({:?})",
        started.elapsed()
    );
    let pid = printed_pid(&result.tail);
    assert!(
        gone_within(pid, Duration::from_secs(3)).await,
        "`sleep 300 &` left behind by a passing test is still running"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_background_process_does_not_outlive_a_failing_command_either() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_test_command(
        dir.path(),
        "sleep 300 & echo pid=$!; exit 3",
        Duration::from_secs(30),
    )
    .await;
    assert_eq!(result.exit_code, 3);
    assert!(gone_within(printed_pid(&result.tail), Duration::from_secs(3)).await);
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_kills_a_running_test_promptly() {
    let dir = tempfile::tempdir().unwrap();
    let stop = StopSignal::new();
    let cancel = stop.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let started = Instant::now();
    let outcome = run_test_command_until(
        dir.path(),
        "sleep 300 & echo pid=$!; sleep 300",
        Duration::from_secs(120),
        &stop,
    )
    .await;
    assert!(matches!(
        outcome,
        TestOutcome::Stopped(StopReason::Cancelled)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel took {:?}",
        started.elapsed()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn disabling_the_assistant_kills_a_running_test_and_its_children() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let stop = StopSignal::new();
    let enabled = Arc::clone(&stop.enabled);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        enabled.store(false, Ordering::SeqCst);
    });
    let command = format!("sleep 300 & echo $! > {}; sleep 300", pid_file.display());
    let started = Instant::now();
    let outcome =
        run_test_command_until(dir.path(), &command, Duration::from_secs(120), &stop).await;
    assert!(matches!(
        outcome,
        TestOutcome::Stopped(StopReason::Disabled)
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(gone_within(pid, Duration::from_secs(3)).await);
}

#[cfg(unix)]
#[tokio::test]
async fn a_timeout_still_reports_as_a_timeout_not_a_stop() {
    let dir = tempfile::tempdir().unwrap();
    let outcome = run_test_command_until(
        dir.path(),
        "sleep 300",
        Duration::from_millis(200),
        &StopSignal::new(),
    )
    .await;
    match outcome {
        TestOutcome::Finished(result) => assert!(result.timed_out),
        TestOutcome::Stopped(reason) => panic!("stopped by {reason:?}"),
    }
}
