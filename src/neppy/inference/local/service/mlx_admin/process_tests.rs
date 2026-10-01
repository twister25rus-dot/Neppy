//! Which recorded processes a spawn marker licenses killing.
//!
//! Hermetic: markers live in a temp dir and the "workers" are `sleep`
//! children of the test, never anything under `~/.neppy`.

use std::path::{Path, PathBuf};

use super::*;

fn marker_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("mlx-t3.spawn")
}

fn sleeper() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("spawn sleep")
}

/// The pid of a process that has already exited.
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("true")
        .spawn()
        .expect("spawn true");
    let pid = child.id();
    child.wait().expect("wait");
    pid
}

fn marker(worker: &std::process::Child, owner: u32, binary: &str, supervised: bool) -> SpawnMarker {
    let mut marker = SpawnMarker::new(worker.id(), Path::new(binary));
    marker.neppy_pid = owner;
    marker.supervised = supervised;
    marker
}

fn reclaim(path: &Path, m: &SpawnMarker) -> bool {
    // `our_pid` is a pid that is neither the owner nor alive-and-ours.
    reclaim_stale_marker(path, m, 1, "t3")
}

#[test]
fn a_marker_for_a_process_that_is_gone_is_just_cleared() {
    let dir = tempfile::tempdir().unwrap();
    let path = marker_path(&dir);
    let mut worker = sleeper();
    let m = marker(&worker, dead_pid(), "/usr/bin/sleep", true);
    write_marker_at(&path, &m).unwrap();
    worker.kill().unwrap();
    worker.wait().unwrap();
    assert!(!reclaim(&path, &m));
    assert!(!path.exists(), "the stale marker is removed");
}

#[test]
fn a_one_shot_cli_server_is_not_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let path = marker_path(&dir);
    let mut worker = sleeper();
    // Written by `neppy-core mlx start`: meant to outlive that CLI.
    let m = marker(&worker, dead_pid(), "/usr/bin/sleep", false);
    write_marker_at(&path, &m).unwrap();
    assert!(!reclaim(&path, &m));
    assert!(worker.try_wait().unwrap().is_none(), "still running");
    assert!(
        path.exists(),
        "its marker is kept for an explicit `mlx stop`"
    );
    worker.kill().unwrap();
    worker.wait().unwrap();
}

#[test]
fn a_server_owned_by_a_live_core_is_not_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let path = marker_path(&dir);
    let mut worker = sleeper();
    // The owner is this test process: alive, and not `our_pid` (1).
    let m = marker(&worker, std::process::id(), "/usr/bin/sleep", true);
    write_marker_at(&path, &m).unwrap();
    assert!(!reclaim(&path, &m));
    assert!(
        worker.try_wait().unwrap().is_none(),
        "the other core's worker lives"
    );
    assert!(path.exists());
    worker.kill().unwrap();
    worker.wait().unwrap();
}

#[test]
fn a_recycled_pid_is_never_killed() {
    let dir = tempfile::tempdir().unwrap();
    let path = marker_path(&dir);
    let mut worker = sleeper();
    // The marker says an MLX server; the pid now belongs to `sleep`.
    let m = marker(&worker, dead_pid(), "/opt/mlx/bin/mlx_vlm.server", true);
    write_marker_at(&path, &m).unwrap();
    assert!(!reclaim(&path, &m));
    assert!(
        worker.try_wait().unwrap().is_none(),
        "an unrelated process survives"
    );
    assert!(
        !path.exists(),
        "the marker is dropped: it describes nothing"
    );
    worker.kill().unwrap();
    worker.wait().unwrap();
}

#[test]
fn a_worker_orphaned_by_a_dead_supervised_core_is_reclaimed() {
    let dir = tempfile::tempdir().unwrap();
    let path = marker_path(&dir);
    let mut worker = sleeper();
    let m = marker(&worker, dead_pid(), "/usr/bin/sleep", true);
    write_marker_at(&path, &m).unwrap();
    assert!(
        reclaim(&path, &m),
        "a verified orphan of a dead supervised core"
    );
    worker.wait().unwrap(); // reaped: it was killed
    assert!(!path.exists());
}
