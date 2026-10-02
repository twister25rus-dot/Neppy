//! `ToolPolicyMiddleware`'s inner-tool check for `use_skill`.
//!
//! A packed tool is withheld from the agent's visible set *before* the session
//! policy is built, so every packed tool is `HideFromPrompt` in that session —
//! including the ones the agent is meant to reach through `use_skill`. The
//! check therefore cannot be "is the inner tool allowed in the session"; it has
//! to tell a tool withheld *into* the pack surface apart from one that was
//! never in scope, and it still has to honour a channel ceiling.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use super::*;
use crate::neppy::agent::tool_policy::AllowAllToolPolicy;
use crate::neppy::tools::agent_policy::ToolPolicyEngine;
use crate::neppy::tools::toolpacks::{
    append_pack_tools, bind_pack_registry, strip_packed_from_visible, USE_SKILL,
};
use crate::neppy::tools::traits::{PermissionLevel, ToolResult};

struct FakeTool(&'static str, PermissionLevel);

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
    fn permission_level(&self) -> PermissionLevel {
        self.1
    }
}

/// Mirror the builder: strip packed names from `visible`, build the session
/// from the stripped set, bind the pack tools to the registry with the
/// withheld names as their reach.
fn middleware(
    agent_id: &str,
    visible: &[&str],
    channel_permissions: HashMap<String, String>,
) -> ToolPolicyMiddleware {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(FakeTool("file_read", PermissionLevel::ReadOnly)),
        Box::new(FakeTool("config_snapshot", PermissionLevel::ReadOnly)),
        Box::new(FakeTool("service_stop", PermissionLevel::Dangerous)),
    ];
    append_pack_tools(&mut tools);
    let mut visible: HashSet<String> = visible.iter().map(|s| s.to_string()).collect();
    let withheld = strip_packed_from_visible(&mut visible, agent_id);
    let session = ToolPolicyEngine::build_session(
        agent_id,
        "web",
        "session",
        &channel_permissions,
        &tools,
        &visible,
    );
    let tools = Arc::new(tools);
    bind_pack_registry(&tools, &withheld);
    ToolPolicyMiddleware::new(
        Arc::new(AllowAllToolPolicy),
        session,
        vec![tools],
        "s".into(),
        "web".into(),
        agent_id.into(),
    )
}

fn use_skill(tool: &str) -> TaToolCall {
    TaToolCall {
        id: "c1".into(),
        name: USE_SKILL.into(),
        arguments: json!({"skill": "system", "tool": tool, "args": {}}),
        invalid: None,
    }
}

#[test]
fn a_withheld_tool_in_scope_is_not_blocked() {
    // The orchestrator's packed disclosure: the tool is hidden from the wire
    // on purpose and must still run through `use_skill`.
    let mw = middleware(
        "orchestrator",
        &["file_read", "config_snapshot"],
        HashMap::new(),
    );
    assert_eq!(
        mw.channel_permission_block(&use_skill("config_snapshot")),
        None
    );
}

#[test]
fn a_tool_outside_the_agents_scope_is_blocked() {
    let mw = middleware(
        "orchestrator",
        &["file_read", "config_snapshot"],
        HashMap::new(),
    );
    assert!(mw
        .channel_permission_block(&use_skill("service_stop"))
        .is_some());
}

#[test]
fn a_channel_ceiling_still_applies_to_a_reachable_tool() {
    // Reach is necessary, not sufficient: a Dangerous packed tool the agent
    // may reach must still be refused on a read-only channel.
    let perms = HashMap::from([("web".to_string(), "read_only".to_string())]);
    let mw = middleware("orchestrator", &["file_read", "service_stop"], perms);
    assert!(mw
        .channel_permission_block(&use_skill("service_stop"))
        .is_some());
}
