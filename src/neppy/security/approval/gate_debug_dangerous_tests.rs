//! Spec 20: a dangerous command in a Debug turn is confirmed by a human every
//! time, even when the user has an "Always allow" rule for `shell`.

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use tempfile::TempDir;

const THREAD: &str = "t-debug-danger";

fn gate() -> (Arc<ApprovalGate>, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let session = format!("session-{}", uuid::Uuid::new_v4());
    (
        Arc::new(ApprovalGate::new(config, session, Duration::from_secs(30))),
        dir,
    )
}

fn debug_turn(dir: &TempDir) -> DebugTurn {
    DebugTurn {
        ctx: DebugCtx::new(dir.path()),
        root: dir.path().to_path_buf(),
        task_id: None,
        checkpoint_id: None,
        settings: DebugModeConfig::default(),
    }
}

fn shell_call(
    gate: &Arc<ApprovalGate>,
    dir: &TempDir,
    debug: bool,
    command: &'static str,
) -> tokio::task::JoinHandle<GateOutcome> {
    let (g, turn) = (gate.clone(), debug.then(|| debug_turn(dir)));
    tokio::spawn(async move {
        let fut = turn_origin::with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: THREAD.into(),
                client_id: "c".into(),
                request_id: None,
            },
            APPROVAL_CHAT_CONTEXT.scope(
                ApprovalChatContext {
                    thread_id: THREAD.into(),
                    client_id: "c".into(),
                },
                async move {
                    g.intercept_audited_raw(
                        "shell",
                        "run",
                        &serde_json::json!({ "command": command }),
                    )
                    .await
                    .0
                },
            ),
        );
        match turn {
            Some(t) => with_turn(t, fut).await,
            None => fut.await,
        }
    })
}

#[tokio::test]
async fn dangerous_debug_command_parks_despite_an_always_allow_rule_but_ls_does_not() {
    let _env = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (gate, dir) = gate();
    let policy = crate::neppy::security::SecurityPolicy {
        auto_approve: vec!["shell".into()],
        ..crate::neppy::security::SecurityPolicy::default()
    };
    let _policy_guard = crate::neppy::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    );

    // Safe command in a Debug turn, and the same dangerous command outside one:
    // both are still auto-allowed by the user's rule.
    for (debug, cmd) in [(true, "ls -la"), (false, "rm -rf build")] {
        let out = tokio::time::timeout(Duration::from_secs(5), shell_call(&gate, &dir, debug, cmd))
            .await
            .expect("must not park")
            .unwrap();
        assert!(matches!(out, GateOutcome::Allow), "{cmd} debug={debug}");
    }
    assert!(gate.list_pending().unwrap().is_empty());

    // Dangerous command inside a Debug turn parks for a human decision.
    let handle = shell_call(&gate, &dir, true, "rm -rf build");
    let mut id = None;
    for _ in 0..2_000 {
        if let Some(found) = gate.pending_all_for_thread(THREAD).first() {
            id = Some(found.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let id = id.expect("the dangerous debug command must park despite the allowlist");
    gate.decide(&id, ApprovalDecision::Deny).unwrap().unwrap();
    let out = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(out, GateOutcome::Deny { .. }));
}
