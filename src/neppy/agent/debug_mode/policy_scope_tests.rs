use super::policy::*;
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::agent::debug_mode::turn;
use crate::neppy::agent::turn_workspace;
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use crate::neppy::security::{AutonomyLevel, SecurityPolicy};

fn cfg() -> DebugModeConfig {
    DebugModeConfig::default()
}

fn kind(d: &DebugCommandDecision) -> &'static str {
    match d {
        DebugCommandDecision::Allow => "allow",
        DebugCommandDecision::Ask(_) => "ask",
        DebugCommandDecision::Deny(_) => "deny",
    }
}

use DebugCommandDecision::{Allow, Deny};

// ── ambient (Debug turn) seams ──────────────────────────────────────────

#[tokio::test]
async fn outside_a_debug_turn_the_gate_is_inert() {
    assert!(turn::current().is_none());
    for c in [
        "git push",
        "sudo rm -rf /",
        "npm install x",
        "git reset --hard",
    ] {
        assert_eq!(gate_current_command(c), Allow, "{c}");
    }
    assert!(git_operation_denied("push").is_none());
    assert!(git_operation_denied("commit").is_none());
    assert!(turn_workspace::extra_roots().is_empty());
}

#[tokio::test]
async fn inside_a_debug_turn_the_gate_and_git_tool_follow_the_settings() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = turn::resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let settings = DebugModeConfig {
        allow_git_commit: false,
        ..cfg()
    };
    turn::run_in_root_with(ws.path(), root, "go", settings, async {
        assert!(matches!(gate_current_command("git push"), Deny(_)));
        assert!(matches!(gate_current_command("git commit -m x"), Deny(_)));
        assert_eq!(kind(&gate_current_command("rm -rf x")), "ask");
        assert_eq!(gate_current_command("cargo test"), Allow);
        assert!(git_operation_denied("push").is_some());
        assert!(git_operation_denied("commit").is_some());
        assert!(git_operation_denied("status").is_none());
        Ok::<_, String>(())
    })
    .await
    .unwrap();
    assert!(turn::current().is_none());
}

#[tokio::test]
async fn a_disabled_debug_mode_refuses_before_any_task_is_recorded() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = turn::resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let off = DebugModeConfig {
        enabled: false,
        ..cfg()
    };
    let ran = std::sync::atomic::AtomicBool::new(false);
    let err = turn::run_in_root_with(ws.path(), root, "go", off, async {
        ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok::<_, String>(())
    })
    .await
    .unwrap_err();
    assert!(err.contains("turned off"), "{err}");
    assert!(
        !ran.load(std::sync::atomic::Ordering::SeqCst),
        "the turn must not run"
    );
    let tasks = crate::neppy::agent::debug_mode::ops::task_list(
        &crate::neppy::agent::debug_mode::ops::DebugCtx::new(ws.path()),
        None,
    )
    .await
    .unwrap()
    .value;
    assert!(tasks.is_empty(), "no task for a refused turn");
}

#[tokio::test]
async fn auto_checkpoint_off_skips_the_checkpoint_but_records_the_task() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = turn::resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let c = DebugModeConfig {
        auto_checkpoint: false,
        ..cfg()
    };
    turn::run_in_root_with(ws.path(), root, "go", c, async { Ok::<_, String>(()) })
        .await
        .unwrap();
    let ctx = crate::neppy::agent::debug_mode::ops::DebugCtx::new(ws.path());
    let tasks = crate::neppy::agent::debug_mode::ops::task_list(&ctx, None)
        .await
        .unwrap()
        .value;
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].checkpoint_id.is_none());
    let cps = crate::neppy::agent::debug_mode::ops::checkpoint_list(&ctx, None)
        .await
        .unwrap()
        .value;
    assert!(cps.is_empty());
}

// ── external directories ────────────────────────────────────────────────

fn policy_for(workspace: &std::path::Path) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: workspace.to_path_buf(),
        action_dir: workspace.to_path_buf(),
        workspace_only: true,
        forbidden_paths: Vec::new(),
        trusted_roots: Vec::new(),
        ..SecurityPolicy::default()
    }
}

#[test]
fn external_path_validation_rejects_unsafe_values() {
    let dir = tempfile::tempdir().unwrap();
    let ok = validate_external_path(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(ok, std::fs::canonicalize(dir.path()).unwrap());
    for bad in ["", "relative/dir", "/", "/definitely/not/here/xyz"] {
        assert!(validate_external_path(bad).is_err(), "{bad:?}");
    }
    let file = dir.path().join("f.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(validate_external_path(file.to_str().unwrap())
        .unwrap_err()
        .contains("not a directory"));
    if let Some(home) = dirs::home_dir() {
        assert!(validate_external_path(home.to_str().unwrap())
            .unwrap_err()
            .contains("too broad"));
    }
    assert!(validate_external_path("/etc").is_err());
    let ssh = dir.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    assert!(validate_external_path(ssh.to_str().unwrap())
        .unwrap_err()
        .contains("protected"));
}

#[test]
fn external_roots_require_the_switch_and_skip_bad_paths() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let off = DebugModeConfig {
        external_paths: vec![p.clone()],
        ..cfg()
    };
    assert!(external_roots(&off).is_empty(), "switch off => nothing");
    let on = DebugModeConfig {
        allow_external_filesystem: true,
        external_paths: vec![p.clone(), p, "/".into(), "relative".into()],
        ..cfg()
    };
    assert_eq!(
        external_roots(&on),
        vec![std::fs::canonicalize(dir.path()).unwrap()],
        "deduplicated, `/` and relative paths dropped"
    );
}

#[tokio::test]
async fn external_roots_are_writable_inside_a_debug_turn_only_and_forbidden_paths_stay_refused() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let sandbox = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let ssh = external.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    let policy = policy_for(sandbox.path());
    let target = external.path().join("notes.md");
    let sibling = other.path().join("notes.md");
    let secret = ssh.join("id_rsa");

    let validate = |p: std::path::PathBuf| {
        let policy = &policy;
        async move { policy.validate_parent_path(p.to_str().unwrap()).await }
    };

    // Baseline: outside any Debug turn the external dir is outside the sandbox.
    assert!(validate(target.clone()).await.is_err());
    assert!(!policy.is_within_trusted_root(&target, true));

    let root = turn::resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let make = |allow: bool| DebugModeConfig {
        allow_external_filesystem: allow,
        external_paths: vec![external.path().to_str().unwrap().to_string()],
        ..cfg()
    };

    // Switch off: still refused inside a Debug turn.
    turn::run_in_root_with(ws.path(), root.clone(), "go", make(false), async {
        assert!(validate(target.clone()).await.is_err());
        Ok::<_, String>(())
    })
    .await
    .unwrap();

    // Switch on: the external dir is read/write, nothing else is.
    turn::run_in_root_with(ws.path(), root, "go", make(true), async {
        let r = validate(target.clone())
            .await
            .expect("external root granted");
        assert!(r.ends_with("notes.md"));
        assert!(policy.is_within_trusted_root(&target, true));
        assert!(
            validate(sibling.clone()).await.is_err(),
            "sibling stays outside"
        );
        assert!(
            validate(secret.clone()).await.is_err(),
            "credential stores stay forbidden inside a granted root"
        );
        assert!(!policy.is_within_trusted_root(&secret, false));
        Ok::<_, String>(())
    })
    .await
    .unwrap();

    // The grant does not leak past the turn.
    assert!(validate(target).await.is_err());
    assert!(turn_workspace::extra_roots().is_empty());
}

#[tokio::test]
async fn propagate_carries_the_debug_turn_and_extra_roots_across_a_spawn() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let external = tempfile::tempdir().unwrap();
    let root = turn::resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let c = DebugModeConfig {
        allow_external_filesystem: true,
        external_paths: vec![external.path().to_str().unwrap().to_string()],
        ..cfg()
    };
    let (in_turn, roots) = turn::run_in_root_with(ws.path(), root, "go", c, async {
        let seen = tokio::spawn(turn_workspace::propagate(async {
            (turn::current().is_some(), turn_workspace::extra_roots())
        }))
        .await
        .unwrap();
        Ok::<_, String>(seen)
    })
    .await
    .unwrap();
    assert!(in_turn);
    assert_eq!(roots.len(), 1);
}
