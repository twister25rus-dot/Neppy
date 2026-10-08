//! The Debug Mode command gate at the `npm_exec` seam: acts only inside a Debug
//! turn, and leaves every other turn exactly as it was.

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
use crate::neppy::agent::host_runtime::NativeRuntime;
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use crate::neppy::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};

fn tool(autonomy: AutonomyLevel) -> NpmExecTool {
    let dir = std::env::temp_dir();
    let security = Arc::new(SecurityPolicy {
        autonomy,
        workspace_dir: dir.clone(),
        action_dir: dir,
        ..SecurityPolicy::default()
    });
    let bootstrap = Arc::new(NodeBootstrap::new(Arc::new(
        crate::neppy::config::Config::default(),
    )));
    NpmExecTool::new(security, Arc::new(NativeRuntime::new()), bootstrap)
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

#[tokio::test]
async fn debug_turn_denies_installs_and_release_scripts_before_anything_runs() {
    let t = tool(AutonomyLevel::Full);
    with_turn(turn(DebugModeConfig::default()), async {
        for (call, needle) in [
            (
                json!({"subcommand": "install", "args": ["left-pad"]}),
                "allow_dependency_install",
            ),
            (
                json!({"subcommand": "run", "args": ["release"]}),
                "Publishing a release",
            ),
            (
                json!({"subcommand": "run", "args": ["release:patch"]}),
                "Publishing a release",
            ),
            (
                json!({"subcommand": "exec", "args": ["--", "bash", "scripts/release-neppy.sh", "1.0.0"]}),
                "Publishing a release",
            ),
        ] {
            let result = t.execute(call.clone()).await.unwrap();
            let out = result.output();
            assert!(result.is_error, "{call}: {out}");
            assert!(
                out.contains(POLICY_BLOCKED_MARKER) && out.contains(needle),
                "{call}: {out}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn outside_a_debug_turn_the_gate_changes_nothing() {
    // Read-only autonomy answers before any runtime is touched, so the verdict
    // shows which layer spoke: the tier's own, never a Debug-flavoured one.
    let t = tool(AutonomyLevel::ReadOnly);
    for call in [
        json!({"subcommand": "install", "args": ["left-pad"]}),
        json!({"subcommand": "run", "args": ["release"]}),
    ] {
        let result = t.execute(call.clone()).await.unwrap();
        let out = result.output();
        assert!(
            result.is_error && out.contains("read-only mode"),
            "{call}: {out}"
        );
        assert!(
            !out.contains("allow_dependency_install") && !out.contains("Publishing a release"),
            "{call}: {out}"
        );
    }
    // The approval hook is the tier's decision, with and without the gate.
    let full = tool(AutonomyLevel::Full);
    let call = json!({"subcommand": "install", "args": ["left-pad"]});
    let outside = full.external_effect_with_args(&call);
    let inside = with_turn(turn(DebugModeConfig::default()), async {
        full.external_effect_with_args(&call)
    })
    .await;
    assert_eq!(outside, inside, "a denial is not an approval prompt");
}
