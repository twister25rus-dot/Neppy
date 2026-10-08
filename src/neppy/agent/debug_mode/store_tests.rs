use super::*;
use crate::neppy::agent::debug_mode::types::*;

fn task(id: &str) -> TaskRecord {
    TaskRecord {
        id: id.into(),
        request: format!("req {id}"),
        created_at: "2026-10-04T00:00:00Z".into(),
        updated_at: "2026-10-04T00:00:00Z".into(),
        status: TaskStatus::Planning,
        files_changed: vec![],
        validation: vec![],
        summary: None,
        checkpoint_id: None,
        branch: None,
        commit: None,
        critical_files: vec![],
        candidate_id: None,
    }
}

fn cp(id: &str) -> Checkpoint {
    Checkpoint {
        id: id.into(),
        description: "d".into(),
        task_id: None,
        project_root: "/p".into(),
        created_at: "2026-10-04T00:00:00Z".into(),
        head: "a".repeat(40),
        branch: Some("main".into()),
        snapshot_sha: "b".repeat(40),
        index_tree: "c".repeat(40),
        dirty_files: vec!["x".into()],
        untracked_files: vec![],
    }
}

#[test]
fn tasks_round_trip_and_survive_a_fresh_store() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    s.task_add(task("t1")).unwrap();
    s.task_add(task("t2")).unwrap();
    s.task_modify("t1", |t| t.status = TaskStatus::Pass)
        .unwrap()
        .unwrap();

    // "Restart": a brand-new store over the same directory.
    let s2 = DebugStore::new(ws.path());
    let list = s2.task_list(10).unwrap();
    assert_eq!(
        list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["t2", "t1"]
    );
    assert_eq!(s2.task_get("t1").unwrap().unwrap().status, TaskStatus::Pass);
    assert!(s2.task_modify("nope", |_| {}).unwrap().is_none());
    assert_eq!(s2.active_task().unwrap().unwrap().id, "t2");
}

#[test]
fn checkpoints_round_trip_newest_first() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    s.checkpoint_add(cp("c1")).unwrap();
    s.checkpoint_add(cp("c2")).unwrap();
    let list = s.checkpoint_list(1).unwrap();
    assert_eq!(list[0].id, "c2");
    assert_eq!(s.checkpoint_get("c1").unwrap().unwrap().dirty_files, ["x"]);
    assert!(s.checkpoint_get("zz").unwrap().is_none());
}

#[test]
fn writes_are_atomic_and_leave_no_temp_files() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    for i in 0..5 {
        s.task_add(task(&format!("t{i}"))).unwrap();
    }
    let names: Vec<String> = std::fs::read_dir(s.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["history.json"], "no .tmp leftovers: {names:?}");
    // The file is always complete, valid JSON.
    let text = std::fs::read_to_string(s.dir().join("history.json")).unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<TaskRecord>>(&text)
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn history_is_capped_to_the_newest_tasks() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    for i in 0..(MAX_TASKS + 3) {
        s.task_add(task(&format!("t{i}"))).unwrap();
    }
    let all = s.task_list(10_000).unwrap();
    assert_eq!(all.len(), MAX_TASKS);
    assert_eq!(all.last().unwrap().id, "t3");
}

#[test]
fn corrupt_history_is_moved_aside_not_fatal() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    std::fs::create_dir_all(s.dir()).unwrap();
    std::fs::write(s.dir().join("history.json"), b"{not json").unwrap();
    assert!(s.task_list(10).unwrap().is_empty());
    s.task_add(task("t1")).unwrap();
    let corrupt = std::fs::read_dir(s.dir())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".corrupt-")
        })
        .count();
    assert_eq!(corrupt, 1);
}

#[test]
fn audit_appends_lines_and_tails_oldest_first() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    for i in 0..5 {
        s.audit_append(&AuditEntry {
            ts: "t".into(),
            op: format!("op{i}"),
            target: "x".into(),
            outcome: "ok".into(),
        })
        .unwrap();
    }
    let tail = s.audit_tail(2).unwrap();
    assert_eq!(
        tail.iter().map(|e| e.op.as_str()).collect::<Vec<_>>(),
        ["op3", "op4"]
    );
    let raw = std::fs::read_to_string(s.dir().join("audit.jsonl")).unwrap();
    assert_eq!(raw.lines().count(), 5);
}

#[test]
fn active_task_is_the_newest_task_and_only_when_it_is_active() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    assert!(s.active_task().unwrap().is_none(), "empty history");

    // An older task stranded in `editing`, then a newer one that finishes.
    let mut old = task("old");
    old.status = TaskStatus::Editing;
    s.task_add(old).unwrap();
    s.task_add(task("new")).unwrap();
    assert_eq!(s.active_task().unwrap().unwrap().id, "new");

    s.task_modify("new", |t| t.status = TaskStatus::Pass)
        .unwrap();
    assert!(
        s.active_task().unwrap().is_none(),
        "the stranded older task must not resurface once the newest finished"
    );

    // Every active status counts while the task is the newest.
    for st in [
        TaskStatus::Planning,
        TaskStatus::Editing,
        TaskStatus::Validating,
    ] {
        s.task_modify("new", |t| t.status = st).unwrap();
        assert_eq!(s.active_task().unwrap().unwrap().id, "new");
    }
}

#[test]
fn abandon_active_except_fails_stale_tasks_with_a_reason() {
    let ws = tempfile::tempdir().unwrap();
    let s = DebugStore::new(ws.path());
    let mut a = task("a");
    a.status = TaskStatus::Editing;
    let mut b = task("b");
    b.status = TaskStatus::Validating;
    b.summary = Some("half done".into());
    let mut done = task("done");
    done.status = TaskStatus::Pass;
    done.summary = Some("shipped".into());
    for t in [a, b, done, task("keep")] {
        s.task_add(t).unwrap();
    }
    let closed = s.abandon_active_except("keep").unwrap();
    assert_eq!(closed, ["a", "b"]);

    let a = s.task_get("a").unwrap().unwrap();
    assert_eq!(a.status, TaskStatus::Failed);
    assert_eq!(a.summary.as_deref(), Some("abandoned: superseded by keep"));
    let b = s.task_get("b").unwrap().unwrap();
    assert_eq!(b.status, TaskStatus::Failed);
    assert_eq!(
        b.summary.as_deref(),
        Some("half done\nabandoned: superseded by keep"),
        "an existing note is kept"
    );
    let done = s.task_get("done").unwrap().unwrap();
    assert_eq!(
        done.status,
        TaskStatus::Pass,
        "terminal tasks are untouched"
    );
    assert_eq!(done.summary.as_deref(), Some("shipped"));
    assert_eq!(
        s.task_get("keep").unwrap().unwrap().status,
        TaskStatus::Planning
    );

    assert!(s.abandon_active_except("keep").unwrap().is_empty());
}
