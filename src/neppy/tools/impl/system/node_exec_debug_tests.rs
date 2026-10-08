//! The Debug Mode source gate at the `node_exec` seam: acts only inside a Debug
//! turn, and leaves every other turn exactly as it was.

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
use crate::neppy::agent::host_runtime::NativeRuntime;
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use crate::neppy::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};

const PUBLISHING_JS: &str =
    "require('child_process').execSync('bash scripts/release-neppy.sh 0.69.0')";

fn tool(autonomy: AutonomyLevel, dir: &std::path::Path) -> NodeExecTool {
    let security = Arc::new(SecurityPolicy {
        autonomy,
        workspace_dir: dir.to_path_buf(),
        action_dir: dir.to_path_buf(),
        ..SecurityPolicy::default()
    });
    let bootstrap = Arc::new(NodeBootstrap::new(Arc::new(
        crate::neppy::config::Config::default(),
    )));
    NodeExecTool::new(
        security,
        Arc::new(NativeRuntime::new()),
        bootstrap,
        crate::neppy::config::RuntimePoolConfig::default(),
        dir.to_path_buf(),
    )
}

fn turn(dir: &std::path::Path) -> DebugTurn {
    DebugTurn {
        ctx: DebugCtx::new(dir),
        root: dir.to_path_buf(),
        task_id: None,
        checkpoint_id: None,
        settings: DebugModeConfig::default(),
    }
}

#[tokio::test]
async fn debug_turn_denies_publish_shaped_inline_code_and_script_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ship.js"), PUBLISHING_JS).unwrap();
    let t = tool(AutonomyLevel::Full, dir.path());
    with_turn(turn(dir.path()), async {
        for call in [
            json!({ "inline_code": PUBLISHING_JS }),
            json!({ "inline_code": "require('child_process').execSync('gh release create v1')" }),
            json!({ "inline_code": "require('child_process').execSync('gh workflow run release-production.yml')" }),
            json!({ "script_path": "ship.js" }),
        ] {
            let result = t.execute(call.clone()).await.unwrap();
            let out = result.output();
            assert!(result.is_error, "{call}: {out}");
            assert!(
                out.contains(POLICY_BLOCKED_MARKER) && out.contains("Publishing a release"),
                "{call}: {out}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn outside_a_debug_turn_the_gate_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ship.js"), PUBLISHING_JS).unwrap();
    // Read-only autonomy answers before any runtime is touched, so the verdict
    // shows which layer spoke: the tier's own, never a Debug-flavoured one.
    let t = tool(AutonomyLevel::ReadOnly, dir.path());
    for call in [
        json!({ "inline_code": PUBLISHING_JS }),
        json!({ "script_path": "ship.js" }),
    ] {
        let result = t.execute(call.clone()).await.unwrap();
        let out = result.output();
        assert!(
            result.is_error && out.contains("read-only mode"),
            "{call}: {out}"
        );
        assert!(!out.contains("Publishing a release"), "{call}: {out}");
    }
}
