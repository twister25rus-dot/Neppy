//! `load_skill` / `use_skill` must stay inside the calling agent's tool scope.
//!
//! The pack tools resolve against the agent's *registry*, and the registry is
//! the full global tool set — a `ToolScope::Named` list only narrows what is
//! visible. So a named agent that lists one packed tool, and is therefore
//! handed `use_skill` in its place, must not be able to reach every other
//! packed tool in the build through it. These tests drive the real
//! `AgentBuilder::build` wiring rather than the pack tools in isolation,
//! because the scope is decided there.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::*;
use crate::neppy::agent::dispatcher::XmlToolDispatcher;
use crate::neppy::memory::Memory;
use crate::neppy::tools::toolpacks::{append_pack_tools, LOAD_SKILL, USE_SKILL};
use crate::neppy::tools::traits::{PermissionLevel, Tool, ToolResult};
use tinyagents::harness::model::{ChatModel, ModelRequest, ModelResponse, ModelStream};

/// Never invoked — the tests call tools directly, not through a turn.
struct NoModel;

#[async_trait]
impl ChatModel<()> for NoModel {
    async fn invoke(&self, _: &(), _: ModelRequest) -> tinyagents::Result<ModelResponse> {
        Err(tinyagents::TinyAgentsError::Model("unused".into()))
    }
    async fn stream(&self, _: &(), _: ModelRequest) -> tinyagents::Result<ModelStream> {
        Err(tinyagents::TinyAgentsError::Model("unused".into()))
    }
}

struct FakeTool(&'static str);

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
        Ok(ToolResult::success(format!("ran {}", self.0)))
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

/// One tool from each of four packs, plus an unpacked one. `config_snapshot`
/// (system) and `wallet_execute_prepared` (crypto) are the escalation targets:
/// config and money movement, reachable by no named agent that did not list
/// them.
const REGISTRY: &[&str] = &[
    "file_read",
    "goal_get",
    "goal_set",
    "config_snapshot",
    "update_check",
    "wallet_execute_prepared",
];

fn build_agent(agent_id: &str, visible: &[&str]) -> Agent {
    crate::neppy::memory::host_impls::install_for_tests();
    let workspace = tempfile::TempDir::new().expect("temp workspace");
    let workspace_path = workspace.path().to_path_buf();
    std::mem::forget(workspace);
    let memory_cfg = crate::neppy::config::MemoryConfig {
        backend: "none".into(),
        ..crate::neppy::config::MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(tinymemory_core::store::create_memory(&memory_cfg, &workspace_path).unwrap());

    let mut tools: Vec<Box<dyn Tool>> = REGISTRY
        .iter()
        .map(|name| Box::new(FakeTool(name)) as Box<dyn Tool>)
        .collect();
    append_pack_tools(&mut tools);

    Agent::builder()
        .chat_model(Arc::new(NoModel))
        .tools(tools)
        .visible_tool_names(
            visible
                .iter()
                .map(|s| s.to_string())
                .collect::<HashSet<_>>(),
        )
        .agent_definition_name(agent_id)
        .memory(mem)
        .tool_dispatcher(Box::new(XmlToolDispatcher))
        .workspace_dir(workspace_path)
        .event_context("pack-scope-session", "pack-scope-channel")
        .build()
        .unwrap()
}

fn tool<'a>(agent: &'a Agent, name: &str) -> &'a dyn Tool {
    agent
        .tools()
        .iter()
        .find(|t| t.name() == name)
        .map(AsRef::as_ref)
        .unwrap_or_else(|| panic!("{name} missing from registry"))
}

async fn use_skill(agent: &Agent, skill: &str, name: &str) -> ToolResult {
    tool(agent, USE_SKILL)
        .execute(json!({"skill": skill, "tool": name, "args": {}}))
        .await
        .unwrap()
}

async fn load_skill(agent: &Agent, skill: &str) -> ToolResult {
    tool(agent, LOAD_SKILL)
        .execute(json!({"skill": skill}))
        .await
        .unwrap()
}

fn text(result: &ToolResult) -> String {
    format!("{:?}", result.content)
}

// ── named agent: closed allowlist ──────────────────────────────────────────

#[tokio::test]
async fn a_named_agent_reaches_the_packed_tool_it_listed() {
    let agent = build_agent("pet_agent", &["file_read", "goal_get"]);
    assert!(
        agent.visible_tool_names.contains(USE_SKILL),
        "precondition: listing a packed tool hands the agent use_skill"
    );
    let result = use_skill(&agent, "goals", "goal_get").await;
    assert!(!result.is_error, "listed tool refused: {}", text(&result));
}

#[tokio::test]
async fn a_named_agent_cannot_reach_other_packs_through_use_skill() {
    let agent = build_agent("pet_agent", &["file_read", "goal_get"]);
    for (skill, name) in [
        ("system", "config_snapshot"),
        ("crypto", "wallet_execute_prepared"),
        ("app_update", "update_check"),
    ] {
        let result = use_skill(&agent, skill, name).await;
        assert!(
            result.is_error,
            "`{name}` is outside pet_agent's allowlist but use_skill ran it: {}",
            text(&result)
        );
        assert!(!text(&result).contains("ran "), "inner tool executed");
    }
}

#[tokio::test]
async fn a_named_agent_cannot_reach_an_unlisted_tool_of_a_listed_pack() {
    // Scope is per tool, not per pack: listing `goal_get` does not grant the
    // write half of the same pack.
    let agent = build_agent("pet_agent", &["file_read", "goal_get"]);
    let result = use_skill(&agent, "goals", "goal_set").await;
    assert!(result.is_error, "goal_set ran through a goal_get grant");
}

#[tokio::test]
async fn a_named_agent_cannot_load_a_pack_outside_its_allowlist() {
    let agent = build_agent("pet_agent", &["file_read", "goal_get"]);
    let result = load_skill(&agent, "system").await;
    assert!(
        result.is_error && !text(&result).contains("config_snapshot"),
        "load_skill disclosed a tool outside the allowlist: {}",
        text(&result)
    );

    let goals = load_skill(&agent, "goals").await;
    assert!(!goals.is_error);
    assert!(text(&goals).contains("goal_get"));
    assert!(
        !text(&goals).contains("goal_set"),
        "load_skill rendered an unlisted tool of a listed pack"
    );
}

// ── orchestrator: packed disclosure unchanged ──────────────────────────────

#[tokio::test]
async fn a_wildcard_agent_reaches_every_withheld_tool() {
    // Empty visible set = wildcard scope; the builder materialises it from the
    // registry, so every packed tool is withheld and every one is reachable.
    let agent = build_agent("orchestrator", &[]);
    for (skill, name) in [
        ("goals", "goal_set"),
        ("system", "config_snapshot"),
        ("app_update", "update_check"),
        ("crypto", "wallet_execute_prepared"),
    ] {
        assert!(
            !agent.visible_tool_names.contains(name),
            "`{name}` should be withheld from the orchestrator's wire surface"
        );
        let result = use_skill(&agent, skill, name).await;
        assert!(
            !result.is_error,
            "orchestrator lost `{name}`: {}",
            text(&result)
        );
    }
    assert!(text(&load_skill(&agent, "system").await).contains("config_snapshot"));
}

#[tokio::test]
async fn a_named_orchestrator_reaches_exactly_its_curated_packed_tools() {
    let agent = build_agent(
        "orchestrator",
        &["file_read", "goal_get", "goal_set", "config_snapshot"],
    );
    assert!(
        !use_skill(&agent, "system", "config_snapshot")
            .await
            .is_error
    );
    assert!(!use_skill(&agent, "goals", "goal_set").await.is_error);
    assert!(
        use_skill(&agent, "crypto", "wallet_execute_prepared")
            .await
            .is_error,
        "the orchestrator's curated list does not name the wallet"
    );
}

// ── settings_agent: owner of the system pack ───────────────────────────────

#[tokio::test]
async fn settings_agent_keeps_its_owned_belt_advertised() {
    let agent = build_agent("settings_agent", &["config_snapshot", "update_check"]);
    assert!(agent.visible_tool_names.contains("config_snapshot"));
    assert!(agent.visible_tool_names.contains("update_check"));
    assert!(
        !agent.visible_tool_names.contains(USE_SKILL),
        "an owner that lost nothing to a pack gained the pack tools"
    );
}

#[tokio::test]
async fn settings_agent_reaches_a_listed_foreign_packed_tool_and_nothing_else() {
    let agent = build_agent("settings_agent", &["config_snapshot", "goal_get"]);
    assert!(agent.visible_tool_names.contains("config_snapshot"));
    assert!(!use_skill(&agent, "goals", "goal_get").await.is_error);
    assert!(
        use_skill(&agent, "crypto", "wallet_execute_prepared")
            .await
            .is_error
    );
}

// ── later narrowing ────────────────────────────────────────────────────────

#[tokio::test]
async fn hide_tools_also_revokes_use_skill_reach() {
    // `hide_tools` is how a caller drops a dangerous tool from an otherwise
    // unchanged belt. A withheld tool is already absent from the visible set,
    // so hiding it must also take it away from `use_skill`.
    let mut agent = build_agent("orchestrator", &[]);
    assert!(
        !use_skill(&agent, "system", "config_snapshot")
            .await
            .is_error
    );
    agent.hide_tools(&["config_snapshot"]);
    assert!(
        use_skill(&agent, "system", "config_snapshot")
            .await
            .is_error,
        "a hidden tool stayed reachable through use_skill"
    );
    assert!(!use_skill(&agent, "goals", "goal_get").await.is_error);
}
