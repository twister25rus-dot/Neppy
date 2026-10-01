use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::neppy::security::policy::AutonomyLevel;
use crate::neppy::security::{TrustedAccess, TrustedRoot};

use super::super::faults::NoFaults;
use super::super::test_support::{spec, store};
use super::*;

struct Env {
    root: tempfile::TempDir,
    ws: tempfile::TempDir,
    store: StateStore,
    _sd: tempfile::TempDir,
    task: String,
    policy: SecurityPolicy,
    cfg: LocalAssistantConfig,
}

fn env() -> Env {
    let root = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let (sd, store) = store();
    let task = store.create_task(&spec("g"), "/p", 8).unwrap().id;
    // The project is a read-write trusted root, as it must be for a task to
    // have been accepted; the default forbidden list (`/tmp`, `/var`, ...) is
    // left in place, which is what the grant has to carve out of.
    let policy = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: ws.path().to_path_buf(),
        trusted_roots: vec![TrustedRoot {
            path: root
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            access: TrustedAccess::ReadWrite,
        }],
        ..SecurityPolicy::default()
    };
    Env {
        root,
        ws,
        store,
        _sd: sd,
        task,
        policy,
        cfg: LocalAssistantConfig::default(),
    }
}

impl Env {
    fn root(&self) -> PathBuf {
        self.root.path().canonicalize().unwrap()
    }

    fn apply(&self, step: u32, idx: usize, edit: &EditOp) -> EditOutcome {
        self.apply_with(step, idx, edit, &NoFaults)
    }

    fn apply_with(&self, step: u32, idx: usize, edit: &EditOp, faults: &dyn Faults) -> EditOutcome {
        let root = self.root();
        let ctx = EditCtx {
            root: &root,
            policy: &self.policy,
            workspace: self.ws.path(),
            cfg: &self.cfg,
            store: &self.store,
            task_id: &self.task,
            step_no: step,
            faults,
        };
        apply_edit(&ctx, idx, edit).unwrap()
    }

    fn write(&self, rel: &str, body: &str) {
        let p = self.root.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.path().join(rel)).unwrap()
    }
}

fn edit(path: &str, search: &str, replace: &str) -> EditOp {
    EditOp {
        path: path.into(),
        search: search.into(),
        replace: replace.into(),
    }
}

fn refused(outcome: &EditOutcome) -> &str {
    match outcome {
        EditOutcome::Refused(why) => why,
        other => panic!("expected refusal, got {other:?}"),
    }
}

#[test]
fn a_unique_search_is_replaced_and_leaves_no_temp_file() {
    let e = env();
    e.write("a.rs", "fn old() {}\n");
    assert_eq!(
        e.apply(1, 0, &edit("a.rs", "old", "new")),
        EditOutcome::Applied
    );
    assert_eq!(e.read("a.rs"), "fn new() {}\n");
    let leftovers: Vec<_> = std::fs::read_dir(e.root.path())
        .unwrap()
        .filter_map(|d| d.ok())
        .filter(|d| d.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn applying_the_same_edit_again_does_nothing() {
    let e = env();
    e.write("a.rs", "one two\n");
    let op = edit("a.rs", "one", "uno");
    assert_eq!(e.apply(1, 0, &op), EditOutcome::Applied);
    assert_eq!(e.apply(1, 0, &op), EditOutcome::AlreadyApplied);
    assert_eq!(e.read("a.rs"), "uno two\n");
}

#[test]
fn a_changed_file_is_a_conflict_and_is_not_overwritten() {
    let e = env();
    e.write("a.rs", "alpha\n");
    // The text the edit expects is gone.
    e.write("a.rs", "beta\n");
    let outcome = e.apply(1, 0, &edit("a.rs", "alpha", "gamma"));
    assert!(matches!(outcome, EditOutcome::Conflict(_)), "{outcome:?}");
    assert_eq!(e.read("a.rs"), "beta\n");
    // Ambiguous search text is also a conflict.
    e.write("b.rs", "x x\n");
    assert!(matches!(
        e.apply(1, 1, &edit("b.rs", "x", "y")),
        EditOutcome::Conflict(_)
    ));
    assert_eq!(e.read("b.rs"), "x x\n");
    let key = effect_key(&e.task, 1, 0, &edit("a.rs", "alpha", "gamma"));
    assert_eq!(
        e.store.get_effect(&key).unwrap().unwrap().status,
        EffectStatus::Conflict
    );
}

#[test]
fn an_empty_search_creates_a_new_file_but_never_replaces_one() {
    let e = env();
    assert_eq!(
        e.apply(1, 0, &edit("new/dir/n.rs", "", "fn n() {}\n")),
        EditOutcome::Applied
    );
    assert_eq!(e.read("new/dir/n.rs"), "fn n() {}\n");
    assert!(matches!(
        e.apply(1, 1, &edit("new/dir/n.rs", "", "clobber")),
        EditOutcome::Conflict(_)
    ));
    assert_eq!(e.read("new/dir/n.rs"), "fn n() {}\n");
}

#[test]
fn bad_paths_are_refused_before_anything_happens() {
    let e = env();
    e.write("ok.rs", "x\n");
    for (n, path) in [
        "/etc/passwd",
        "../escape.rs",
        "a/../../b.rs",
        "./x.rs",
        "",
        "~/x",
        "a\\b",
    ]
    .iter()
    .enumerate()
    {
        let outcome = e.apply(1, n, &edit(path, "x", "y"));
        refused(&outcome);
    }
    assert_eq!(e.read("ok.rs"), "x\n");
}

#[test]
fn git_internals_ignored_and_generated_files_are_refused() {
    let e = env();
    assert!(std::process::Command::new("git")
        .arg("-C")
        .arg(e.root.path())
        .args(["init", "-q"])
        .status()
        .unwrap()
        .success());
    e.write(".gitignore", "secret.txt\n");
    e.write("secret.txt", "s\n");
    e.write("target/x.rs", "x\n");
    e.write("gen.rs", "// @generated\nx\n");
    e.write("Cargo.lock", "x\n");
    e.write("vendor/a/vendor/d.rs", "x\n");
    for (n, path) in [
        ".git/config",
        "secret.txt",
        "target/x.rs",
        "gen.rs",
        "Cargo.lock",
        "vendor/a/vendor/d.rs",
    ]
    .iter()
    .enumerate()
    {
        let outcome = e.apply(1, n, &edit(path, "x", "y"));
        refused(&outcome);
    }
    assert_eq!(e.read("secret.txt"), "s\n");
    assert_eq!(e.read("gen.rs"), "// @generated\nx\n");
}

#[cfg(unix)]
#[test]
fn a_symlink_anywhere_on_the_path_is_refused() {
    let e = env();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("t.rs"), "x\n").unwrap();
    std::os::unix::fs::symlink(outside.path().join("t.rs"), e.root.path().join("link.rs")).unwrap();
    std::os::unix::fs::symlink(outside.path(), e.root.path().join("dirlink")).unwrap();
    assert!(refused(&e.apply(1, 0, &edit("link.rs", "x", "y"))).contains("symlink"));
    assert!(refused(&e.apply(1, 1, &edit("dirlink/t.rs", "x", "y"))).contains("symlink"));
    assert_eq!(
        std::fs::read_to_string(outside.path().join("t.rs")).unwrap(),
        "x\n"
    );
}

#[test]
fn the_neppy_workspace_is_never_editable() {
    let e = env();
    // A project that contains the workspace must still not be able to touch it.
    let inner_ws = e.root.path().join("ws");
    std::fs::create_dir_all(&inner_ws).unwrap();
    std::fs::write(inner_ws.join("state.db"), "x").unwrap();
    let mut e = e;
    e.policy.workspace_dir = inner_ws.clone();
    let root = e.root();
    let ctx = EditCtx {
        root: &root,
        policy: &e.policy,
        workspace: &inner_ws,
        cfg: &e.cfg,
        store: &e.store,
        task_id: &e.task,
        step_no: 1,
        faults: &NoFaults,
    };
    let outcome = apply_edit(&ctx, 0, &edit("ws/state.db", "x", "y")).unwrap();
    assert!(refused(&outcome).contains("workspace"));
    assert_eq!(
        std::fs::read_to_string(inner_ws.join("state.db")).unwrap(),
        "x"
    );
}

#[test]
fn a_read_only_tier_refuses_every_edit() {
    let mut e = env();
    e.policy.autonomy = AutonomyLevel::ReadOnly;
    e.write("a.rs", "x\n");
    assert!(refused(&e.apply(1, 0, &edit("a.rs", "x", "y"))).contains("read-only"));
    assert_eq!(e.read("a.rs"), "x\n");
}

struct CrashAt(FaultPoint, AtomicUsize);

impl Faults for CrashAt {
    fn crash_at(&self, point: FaultPoint) -> bool {
        if point == self.0 {
            self.1.fetch_add(1, Ordering::SeqCst);
            return true;
        }
        false
    }
}

fn crash(point: FaultPoint) -> CrashAt {
    CrashAt(point, AtomicUsize::new(0))
}

fn crashed(e: &Env, step: u32, idx: usize, op: &EditOp, point: FaultPoint) {
    let root = e.root();
    let faults = crash(point);
    let ctx = EditCtx {
        root: &root,
        policy: &e.policy,
        workspace: e.ws.path(),
        cfg: &e.cfg,
        store: &e.store,
        task_id: &e.task,
        step_no: step,
        faults: &faults,
    };
    let err = apply_edit(&ctx, idx, op).unwrap_err();
    assert!(matches!(err, AssistantError::Interrupted(_)));
    assert_eq!(faults.1.load(Ordering::SeqCst), 1);
}

#[test]
fn a_crash_between_intent_and_write_resumes_by_writing_once() {
    let e = env();
    e.write("a.rs", "fn old() {}\n");
    let op = edit("a.rs", "old", "new");
    crashed(&e, 1, 0, &op, FaultPoint::Intent(0));
    assert_eq!(e.read("a.rs"), "fn old() {}\n", "nothing written yet");
    assert_eq!(e.apply(1, 0, &op), EditOutcome::Applied);
    assert_eq!(e.read("a.rs"), "fn new() {}\n");
    assert_eq!(e.apply(1, 0, &op), EditOutcome::AlreadyApplied);
}

#[test]
fn a_crash_between_write_and_mark_resumes_without_writing_again() {
    let e = env();
    e.write("a.rs", "fn old() {}\n");
    let op = edit("a.rs", "old", "new");
    crashed(&e, 1, 0, &op, FaultPoint::Write(0));
    assert_eq!(e.read("a.rs"), "fn new() {}\n");
    let key = effect_key(&e.task, 1, 0, &op);
    assert_eq!(
        e.store.get_effect(&key).unwrap().unwrap().status,
        EffectStatus::Intent
    );
    // A replace that contains its own search text would double-apply if it
    // ran twice; this one would fail to find `old` and conflict. Neither may
    // happen: the ledger recognises the written file.
    assert_eq!(e.apply(1, 0, &op), EditOutcome::AlreadyApplied);
    assert_eq!(e.read("a.rs"), "fn new() {}\n");
    assert_eq!(
        e.store.get_effect(&key).unwrap().unwrap().status,
        EffectStatus::Applied
    );
}

#[test]
fn a_crash_then_an_outside_change_is_a_conflict_not_an_overwrite() {
    let e = env();
    e.write("a.rs", "fn old() {}\n");
    let op = edit("a.rs", "old", "new");
    crashed(&e, 1, 0, &op, FaultPoint::Intent(0));
    e.write("a.rs", "fn someone_elses_edit() {}\n");
    assert!(matches!(e.apply(1, 0, &op), EditOutcome::Conflict(_)));
    assert_eq!(e.read("a.rs"), "fn someone_elses_edit() {}\n");
}

#[test]
fn keys_differ_by_position_and_content_but_not_by_call() {
    let a = edit("a.rs", "x", "y");
    let b = edit("a.rs", "x", "z");
    assert_eq!(effect_key("t", 1, 0, &a), effect_key("t", 1, 0, &a));
    assert_ne!(effect_key("t", 1, 0, &a), effect_key("t", 1, 1, &a));
    assert_ne!(effect_key("t", 1, 0, &a), effect_key("t", 2, 0, &a));
    assert_ne!(effect_key("t", 1, 0, &a), effect_key("t", 1, 0, &b));
    let _ = Path::new("");
}

// ---- review fixes --------------------------------------------------------

#[test]
fn files_that_run_code_on_their_own_are_refused_by_default() {
    let e = env();
    for (n, path) in [
        ".husky/pre-commit",
        ".githooks/pre-push",
        ".vscode/tasks.json",
        ".idea/runConfigurations/x.xml",
        ".github/workflows/ci.yml",
        ".cargo/config.toml",
        "build.rs",
        ".envrc",
        "package.json",
    ]
    .iter()
    .enumerate()
    {
        let outcome = e.apply(1, n, &edit(path, "", "echo planted\n"));
        assert!(
            refused(&outcome).contains("allow_sensitive_paths"),
            "{path}: {outcome:?}"
        );
        assert!(!e.root.path().join(path).exists(), "{path} was written");
    }
    // An ordinary file in the same tree is fine.
    assert_eq!(
        e.apply(1, 99, &edit("src/ok.rs", "", "fn ok() {}\n")),
        EditOutcome::Applied
    );
}

#[test]
fn sensitive_paths_can_be_enabled_in_config() {
    let mut e = env();
    e.cfg.allow_sensitive_paths = true;
    assert_eq!(
        e.apply(1, 0, &edit(".husky/pre-commit", "", "#!/bin/sh\n")),
        EditOutcome::Applied
    );
    assert_eq!(e.read(".husky/pre-commit"), "#!/bin/sh\n");
    // `.git` stays off limits whatever the setting.
    refused(&e.apply(1, 1, &edit(".git/hooks/pre-commit", "", "x")));
}

#[test]
fn a_forbidden_path_inside_the_project_is_honoured_for_each_edit() {
    let mut e = env();
    let vault = e.root().join("vault");
    e.write("vault/key.txt", "k\n");
    e.policy.forbidden_paths = vec![vault.to_string_lossy().into_owned()];
    let outcome = e.apply(1, 0, &edit("vault/key.txt", "k", "x"));
    assert!(refused(&outcome).contains("forbidden"), "{outcome:?}");
    assert_eq!(e.read("vault/key.txt"), "k\n");
}

#[test]
fn an_edit_is_refused_once_the_write_grant_is_gone() {
    let mut e = env();
    e.write("a.rs", "one\n");
    e.policy.trusted_roots.clear();
    // With the grant gone the default forbidden list (which covers the scratch
    // directory the test runs in) applies again, and the path is refused.
    let outcome = e.apply(1, 0, &edit("a.rs", "one", "two"));
    assert!(!refused(&outcome).is_empty(), "{outcome:?}");
    assert_eq!(e.read("a.rs"), "one\n");
}

#[test]
fn an_applied_edit_stays_applied_when_the_resume_would_refuse_its_path() {
    let mut e = env();
    e.write("a.rs", "one\n");
    let op = edit("a.rs", "one", "two");
    assert_eq!(e.apply(1, 0, &op), EditOutcome::Applied);
    let key = effect_key(&e.task, 1, 0, &op);

    // Between the crash and the resume the grant is withdrawn, so the path
    // would now be refused. The edit already landed.
    e.policy.trusted_roots.clear();
    assert_eq!(e.apply(1, 0, &op), EditOutcome::AlreadyApplied);
    let row = e.store.get_effect(&key).unwrap().unwrap();
    assert_eq!(
        row.status,
        EffectStatus::Applied,
        "not overwritten to refused"
    );
    assert!(row.result.is_none());
    assert_eq!(e.read("a.rs"), "two\n");
}

#[test]
fn a_conflict_also_survives_a_policy_change() {
    let mut e = env();
    e.write("a.rs", "beta\n");
    let op = edit("a.rs", "alpha", "gamma");
    assert!(matches!(e.apply(1, 0, &op), EditOutcome::Conflict(_)));
    e.policy.trusted_roots.clear();
    assert!(matches!(e.apply(1, 0, &op), EditOutcome::Conflict(_)));
}

#[test]
fn git_ignore_rules_apply_when_the_root_is_a_subdirectory_of_the_repository() {
    let e = env();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(e.root.path())
            .args(args)
            .status()
            .unwrap()
            .success());
    };
    git(&["init", "-q"]);
    e.write(".gitignore", "services/api/secret.txt\n*.log\n");
    e.write("services/api/secret.txt", "s\n");
    e.write("services/api/ok.txt", "o\n");
    let sub = e.root().join("services/api");
    assert!(!sub.join(".git").exists(), "the root is not the repo top");

    assert!(
        git_ignored(&sub, "secret.txt"),
        "ignored from a subdirectory"
    );
    assert!(git_ignored(&sub, "debug.log"));
    assert!(!git_ignored(&sub, "ok.txt"));
    assert!(!git_ignored(&sub, "src/new.rs"));
    // Not a repository at all: no rule applies.
    let plain = tempfile::tempdir().unwrap();
    assert!(!git_ignored(plain.path(), "secret.txt"));
}

#[test]
fn an_intent_whose_write_landed_is_applied_even_if_the_path_is_refused_on_resume() {
    use super::super::faults::{FaultPoint, Faults};

    struct CrashAfterWrite;
    impl Faults for CrashAfterWrite {
        fn crash_at(&self, point: FaultPoint) -> bool {
            point == FaultPoint::Write(0)
        }
    }

    let mut e = env();
    e.write("a.rs", "one\n");
    let op = edit("a.rs", "one", "two");
    // The process dies after the file is written but before the row says so.
    let crashed = {
        let root = e.root();
        let ctx = EditCtx {
            root: &root,
            policy: &e.policy,
            workspace: e.ws.path(),
            cfg: &e.cfg,
            store: &e.store,
            task_id: &e.task,
            step_no: 1,
            faults: &CrashAfterWrite,
        };
        apply_edit(&ctx, 0, &op)
    };
    assert!(crashed.is_err(), "the simulated crash");
    assert_eq!(e.read("a.rs"), "two\n");
    let key = effect_key(&e.task, 1, 0, &op);
    assert_eq!(
        e.store.get_effect(&key).unwrap().unwrap().status,
        EffectStatus::Intent
    );

    // On resume the grant is gone. The edit is on disk: it is applied, not refused.
    e.policy.trusted_roots.clear();
    assert_eq!(e.apply(1, 0, &op), EditOutcome::AlreadyApplied);
    assert_eq!(
        e.store.get_effect(&key).unwrap().unwrap().status,
        EffectStatus::Applied
    );
}
