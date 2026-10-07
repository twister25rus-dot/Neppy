//! The Debug Mode command gate at the `shell` tool seam: acts only inside a
//! Debug turn, and is byte-identical to the tier's own decision everywhere else.

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
use crate::neppy::agent::host_runtime::NativeRuntime;
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use crate::neppy::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};

fn tool() -> ShellTool {
    let dir = std::env::temp_dir();
    ShellTool::new(
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: dir.clone(),
            action_dir: dir,
            ..SecurityPolicy::default()
        }),
        Arc::new(NativeRuntime::new()),
        AuditLogger::disabled(),
    )
}

fn turn(settings: DebugModeConfig) -> DebugTurn {
    DebugTurn {
        ctx: DebugCtx::new(&std::env::temp_dir()),
        root: std::env::temp_dir(),
        task_id: None,
        checkpoint_id: None,
        settings,
    }
}

fn effect(t: &ShellTool, cmd: &str) -> bool {
    t.external_effect_with_args(&json!({ "command": cmd }))
}

#[tokio::test]
async fn debug_denials_come_back_as_policy_blocked_tool_errors() {
    let t = tool();
    with_turn(turn(DebugModeConfig::default()), async {
        for (cmd, needle) in [
            ("npm install left-pad", "allow_dependency_install"),
            ("git push origin main", "allow_git_push"),
            ("sudo true", "allow_system_commands"),
        ] {
            let (allowed, result) = t.run_with_security(cmd, None).await;
            assert!(!allowed && result.is_error, "{cmd}");
            let out = result.output();
            assert!(
                out.contains(POLICY_BLOCKED_MARKER) && out.contains(needle),
                "{cmd}: {out}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn debug_ask_routes_through_the_existing_approval_gate() {
    let t = tool();
    // Full autonomy lets a branch delete run unprompted ...
    assert!(
        !effect(&t, "git branch -D old"),
        "tier decision outside a Debug turn"
    );
    // ... but a Debug turn asks for it, through the same `external_effect` hook.
    with_turn(turn(DebugModeConfig::default()), async {
        assert!(effect(&t, "git branch -D old"));
        assert!(!effect(&t, "ls -la"), "safe commands never prompt");
    })
    .await;
    let lax = DebugModeConfig {
        dangerous_commands_require_confirmation: false,
        ..DebugModeConfig::default()
    };
    with_turn(turn(lax), async {
        assert!(!effect(&t, "git branch -D old"));
    })
    .await;
}

#[tokio::test]
async fn outside_a_debug_turn_the_gate_changes_nothing() {
    let t = tool();
    // The same commands a Debug turn would deny get no Debug-flavoured verdict.
    for cmd in ["npm install left-pad", "git push", "sudo true"] {
        assert_eq!(
            effect(&t, cmd),
            t.security.gate_decision(t.security.classify_command(cmd)) == GateDecision::Prompt,
            "{cmd}: approval decision must equal the tier's own"
        );
    }
    use crate::neppy::agent::debug_mode::policy::{gate_current_command, DebugCommandDecision};
    assert_eq!(
        gate_current_command("git push"),
        DebugCommandDecision::Allow
    );
}
