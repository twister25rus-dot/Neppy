//! The fake worker is `sleep 300`; every marker lives in a temp dir, never in
//! the real `~/.neppy`.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use tempfile::TempDir;

use super::super::super::spawn_marker::{read_marker_at, write_marker_at, OllamaSpawnMarker};
use super::*;

const GRACE: Duration = Duration::from_millis(1500);

fn sleeper() -> Child {
    Command::new("sleep")
        .arg("300")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sleep")
}

/// A shell that ignores SIGTERM (and passes the ignore on to `sleep`).
///
/// Returns only once the trap is installed: signalling it earlier would kill it
/// with the default disposition and the test would pass or fail on timing.
fn term_deaf() -> Child {
    use std::io::{BufRead, BufReader};
    let mut child = Command::new("sh")
        .args(["-c", "trap '' TERM; echo ready; sleep 300"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sh");
    let mut line = String::new();
    BufReader::new(child.stdout.as_mut().unwrap())
        .read_line(&mut line)
        .expect("read ready");
    assert_eq!(line.trim(), "ready");
    child
}

fn dead_pid() -> u32 {
    let mut child = Command::new("true").spawn().expect("spawn true");
    let pid = child.id();
    child.wait().expect("wait true");
    pid
}

fn marker(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("mlx-{id}.spawn"))
}

fn write(path: &Path, pid: u32, owner: u32, supervised: bool, binary: &str) {
    let mut m = OllamaSpawnMarker::new(pid, Path::new(binary));
    m.neppy_pid = owner;
    m.supervised = supervised;
    write_marker_at(path, &m).expect("write marker");
}

fn tracked(pid: u32, marker_path: PathBuf, binary_name: Option<&str>) -> Tracked {
    Tracked {
        pid,
        marker_path,
        binary_name: binary_name.map(str::to_string),
    }
}

fn finish(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn a_cooperative_worker_is_stopped_by_sigterm_and_its_marker_cleared() {
    let dir = TempDir::new().unwrap();
    let path = marker(dir.path(), "coop");
    let mut child = sleeper();
    let pid = child.id();
    write(&path, pid, std::process::id(), true, "sleep");

    let stopped = terminate_blocking(&[tracked(pid, path.clone(), Some("sleep"))], GRACE);

    assert_eq!(stopped, 1);
    // Reaping is the owner's job; once reaped the pid must be gone.
    let status = child.wait().expect("wait");
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(status.signal(), Some(libc::SIGTERM), "stopped gracefully");
    assert!(!pid_running(pid));
    assert!(read_marker_at(&path).is_none(), "marker must be cleared");
}

#[test]
fn a_worker_that_ignores_sigterm_is_killed_after_the_grace() {
    let dir = TempDir::new().unwrap();
    let path = marker(dir.path(), "deaf");
    let mut child = term_deaf();
    let pid = child.id();
    write(&path, pid, std::process::id(), true, "sh");

    let started = std::time::Instant::now();
    let stopped = terminate_blocking(
        &[tracked(pid, path.clone(), None)],
        Duration::from_millis(200),
    );

    assert_eq!(stopped, 1);
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "must stay bounded"
    );
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        child.wait().expect("wait").signal(),
        Some(libc::SIGKILL),
        "escalated to SIGKILL"
    );
    assert!(read_marker_at(&path).is_none());
}

#[test]
fn a_pid_that_no_longer_runs_our_binary_is_not_signalled() {
    let dir = TempDir::new().unwrap();
    let path = marker(dir.path(), "reused");
    let child = sleeper();
    let pid = child.id();
    write(&path, pid, std::process::id(), true, "mlx_vlm.server");

    let stopped = terminate_blocking(&[tracked(pid, path.clone(), Some("mlx_vlm.server"))], GRACE);

    assert_eq!(stopped, 0);
    assert!(pid_running(pid), "an unrelated process must survive");
    assert!(read_marker_at(&path).is_none(), "the stale marker goes");
    finish(child);
}

#[test]
fn shutdown_stops_every_tracked_worker_without_a_pool_or_config() {
    let dir = TempDir::new().unwrap();
    let (pa, pb) = (marker(dir.path(), "reg-a"), marker(dir.path(), "reg-b"));
    let (mut a, mut b) = (sleeper(), sleeper());
    write(&pa, a.id(), std::process::id(), true, "sleep");
    write(&pb, b.id(), std::process::id(), true, "sleep");
    track(a.id(), &pa, Path::new("/opt/bin/sleep"));
    track(b.id(), &pb, Path::new("/opt/bin/sleep"));

    let stopped = shutdown_tracked_workers_blocking(GRACE);

    assert_eq!(stopped, 2);
    a.wait().unwrap();
    b.wait().unwrap();
    assert!(!pid_running(a.id()) && !pid_running(b.id()));
    assert!(read_marker_at(&pa).is_none() && read_marker_at(&pb).is_none());
    assert_eq!(tracked_count(), 0, "registry is drained");
    assert_eq!(
        shutdown_tracked_workers_blocking(GRACE),
        0,
        "a second shutdown is a harmless no-op"
    );
}

#[tokio::test]
async fn the_pool_stops_the_workers_it_holds_on_shutdown() {
    use super::super::pool::MlxPool;
    use super::super::process::MlxProcess;

    let child = tokio::process::Command::new("sleep")
        .arg("300")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sleep");
    let pid = child.id().expect("pid");
    let pool = MlxPool::new();
    pool.insert_for_test(MlxProcess::from_child_for_test(
        "reaper-pool-test",
        0,
        child,
    ))
    .await;
    assert!(pool.is_running("reaper-pool-test").await);

    let stopped = pool.shutdown_all(GRACE).await;

    assert_eq!(stopped, 1);
    assert!(!pid_running(pid), "the worker must not survive shutdown");
    assert!(pool.pid_of("reaper-pool-test").await.is_none());
    assert_eq!(pool.shutdown_all(GRACE).await, 0, "idempotent");
}

#[test]
fn a_dead_supervised_hosts_worker_is_reaped_on_the_next_boot() {
    let dir = TempDir::new().unwrap();
    let path = marker(dir.path(), "orphan");
    let mut worker = sleeper();
    write(&path, worker.id(), dead_pid(), true, "/usr/bin/sleep");

    let reaped = reap_stale_in(dir.path(), std::process::id());

    assert_eq!(reaped, 1);
    worker.wait().unwrap();
    assert!(!pid_running(worker.id()));
    assert!(read_marker_at(&path).is_none());
}

#[test]
fn reaping_leaves_alone_what_it_does_not_own() {
    let dir = TempDir::new().unwrap();
    let (cli, live, recycled, ours) = (
        marker(dir.path(), "cli"),
        marker(dir.path(), "live"),
        marker(dir.path(), "recycled"),
        marker(dir.path(), "ours"),
    );
    let (w_cli, w_live, w_recycled, w_ours) = (sleeper(), sleeper(), sleeper(), sleeper());
    // A one-shot CLI start: owner is gone, but it was never supervised.
    write(&cli, w_cli.id(), dead_pid(), false, "sleep");
    // Owned by a core that is still running (stand in with this test process).
    write(&live, w_live.id(), std::process::id(), true, "sleep");
    // Owner gone, but the pid now runs something else.
    write(
        &recycled,
        w_recycled.id(),
        dead_pid(),
        true,
        "mlx_vlm.server",
    );
    // Written by the process doing the reaping.
    write(&ours, w_ours.id(), 4_000_000, true, "sleep");

    let reaped = reap_stale_in(dir.path(), 4_000_000);

    assert_eq!(reaped, 0);
    for w in [&w_cli, &w_live, &w_recycled, &w_ours] {
        assert!(pid_running(w.id()), "pid {} must survive", w.id());
    }
    assert!(read_marker_at(&cli).is_some(), "CLI marker kept for `stop`");
    assert!(read_marker_at(&live).is_some());
    assert!(read_marker_at(&ours).is_some());
    assert!(
        read_marker_at(&recycled).is_none(),
        "a marker naming a recycled pid is dropped, the process is not"
    );
    for w in [w_cli, w_live, w_recycled, w_ours] {
        finish(w);
    }
}

#[test]
fn a_stale_marker_for_a_dead_worker_is_just_cleared() {
    let dir = TempDir::new().unwrap();
    let path = marker(dir.path(), "gone");
    write(&path, dead_pid(), dead_pid(), true, "sleep");

    assert_eq!(reap_stale_in(dir.path(), std::process::id()), 0);
    assert!(read_marker_at(&path).is_none());
}
