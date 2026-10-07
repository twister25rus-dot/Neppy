use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use crate::neppy::security::policy::SecurityPolicy;

fn policy_in(root: &Path) -> SecurityPolicy {
    SecurityPolicy {
        action_dir: root.to_path_buf(),
        ..SecurityPolicy::default()
    }
}

fn debug_turn(root: &Path, ws: &Path) -> DebugTurn {
    DebugTurn {
        ctx: DebugCtx::new(ws),
        root: root.to_path_buf(),
        task_id: None,
        checkpoint_id: None,
        settings: DebugModeConfig::default(),
    }
}

#[tokio::test]
async fn debug_turn_refuses_writes_to_the_recovery_script_only() {
    let (repo, ws) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir_all(repo.path().join("scripts")).unwrap();
    std::fs::write(repo.path().join("scripts/neppy-recover.sh"), "#!/bin/sh\n").unwrap();
    let policy = policy_in(repo.path());
    let target = repo.path().join("scripts/neppy-recover.sh");
    let target = target.to_str().unwrap();

    // Outside a Debug turn: unchanged (scoping the root alone is not enough).
    let outside = turn_workspace::with_workspace(repo.path().to_path_buf(), async {
        policy.validate_parent_path(target).await
    })
    .await;
    assert!(outside.is_ok(), "{outside:?}");

    let (blocked, other, read) = turn_workspace::with_workspace(
        repo.path().to_path_buf(),
        with_turn(debug_turn(repo.path(), ws.path()), async {
            (
                policy.validate_parent_path(target).await,
                policy
                    .validate_parent_path(repo.path().join("scripts/other.sh").to_str().unwrap())
                    .await,
                policy.validate_path(target).await,
            )
        }),
    )
    .await;
    let e = blocked.unwrap_err();
    assert!(e.contains("protected in Debug mode"), "{e}");
    assert!(e.contains(POLICY_BLOCKED_MARKER), "{e}");
    assert!(other.is_ok(), "other files stay writable: {other:?}");
    assert!(read.is_ok(), "reads stay allowed: {read:?}");
}
