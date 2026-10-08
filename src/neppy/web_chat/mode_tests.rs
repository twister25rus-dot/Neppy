use super::*;
use crate::neppy::agent::harness::AgentDefinitionRegistry;
use crate::neppy::agent::profiles::AgentProfileStore;
use crate::neppy::inference::turn_controls::TurnModelControls;
use std::collections::HashSet;

const ORCHESTRATOR_TOML: &str = include_str!("../agent/registry/agents/orchestrator/agent.toml");

fn names_tool(toml: &str, name: &str) -> bool {
    let bare = format!("\"{name}\"");
    let trailing = format!("\"{name}\",");
    toml.lines()
        .map(str::trim)
        .any(|line| line == bare || line == trailing)
}

/// A real orchestrator session agent built for `mode`, exactly as a chat turn
/// builds it, plus the set of tool names the model can see.
fn visible_for(mode: ThreadMode) -> HashSet<String> {
    AgentDefinitionRegistry::init_global_builtins().expect("builtins");
    let workspace = tempfile::TempDir::new().expect("tempdir");
    let config = crate::neppy::config::Config {
        workspace_dir: workspace.path().to_path_buf(),
        action_dir: workspace.path().to_path_buf(),
        ..crate::neppy::config::Config::default()
    };
    let (_, profile) = AgentProfileStore::new(config.workspace_dir.clone())
        .resolve(None)
        .expect("default profile");
    let agent = super::super::session::build_session_agent(
        &config,
        "client-1",
        "thread-mode-test",
        MODE_AWARE_AGENT_ID,
        &profile,
        None,
        None,
        TurnModelControls::default(),
        None,
        Some(mode),
    )
    .expect("build orchestrator session agent");
    let visible = agent.visible_tool_names_for_test();
    if visible.is_empty() {
        agent.tool_specs().iter().map(|s| s.name.clone()).collect()
    } else {
        visible.clone()
    }
}

#[test]
fn chat_hidden_list_names_the_fleet_but_never_the_capability_helpers() {
    for fleet in [
        "spawn_async_subagent",
        "spawn_parallel_agents",
        "list_subagents",
        "steer_subagent",
        "close_subagent",
        "wait_subagent",
    ] {
        assert!(
            CHAT_HIDDEN_TOOLS.contains(&fleet),
            "{fleet} must be hidden in chat"
        );
    }
    // Access to MCP / memory / skills must survive chat mode, and so must the
    // resume path for a blocking helper that paused on a question.
    for kept in [
        "continue_subagent",
        "delegate_use_mcp_server",
        "delegate_retrieve_memory",
        "memory_recall",
        "mcp_registry_status",
        "run_workflow",
        "skill_registry_search",
    ] {
        assert!(
            !CHAT_HIDDEN_TOOLS.contains(&kept),
            "{kept} must stay available in chat mode"
        );
    }
}

#[test]
fn every_orchestration_only_tool_is_hidden_in_chat_and_named_by_the_orchestrator() {
    for tool in ORCHESTRATION_ONLY_TOOLS {
        assert!(
            CHAT_HIDDEN_TOOLS.contains(tool),
            "{tool} is orchestration-only so chat must hide it"
        );
        assert!(
            names_tool(ORCHESTRATOR_TOML, tool),
            "orchestrator agent.toml must name {tool} so Orchestration mode has it"
        );
    }
    // continue_subagent is shared by both modes; `wait`/`wait_loop` stay retired.
    assert!(names_tool(ORCHESTRATOR_TOML, "continue_subagent"));
    assert!(!names_tool(ORCHESTRATOR_TOML, "wait"));
    assert!(!names_tool(ORCHESTRATOR_TOML, "wait_loop"));
}

#[test]
fn only_the_orchestrator_implements_modes() {
    assert_eq!(
        effective_mode("orchestrator", ThreadMode::Orchestration),
        Some(ThreadMode::Orchestration)
    );
    assert_eq!(
        effective_mode("orchestrator", ThreadMode::Chat),
        Some(ThreadMode::Chat)
    );
    assert_eq!(
        effective_mode("researcher", ThreadMode::Orchestration),
        None
    );
}

#[test]
fn addenda_describe_their_mode_and_the_recovery_policy() {
    let chat = prompt_addendum(ThreadMode::Chat);
    assert!(chat.contains("Chat mode"));
    assert!(chat.contains("Do not spawn workers"));
    assert!(chat.contains("continue_subagent"));

    let orch = prompt_addendum(ThreadMode::Orchestration);
    assert!(orch.contains("Orchestration mode"));
    for needle in [
        "Decompose",
        "Sequential or parallel",
        "Assign roles and models",
        "Monitor",
        "Recovery policy",
        "Retry once",
        "Respawn with a narrowed prompt",
        "Split or skip",
        "Escalate to the user",
        "Resolve conflicts by evidence",
        "Validate before reporting",
        "steer_subagent",
        "close_subagent",
        "wait_subagent",
        "continue_subagent",
    ] {
        assert!(
            orch.contains(needle),
            "supervisor addendum missing `{needle}`"
        );
    }
    // Every tool the supervisor is told to use must actually be one it has.
    for tool in [
        "steer_subagent",
        "close_subagent",
        "wait_subagent",
        "spawn_async_subagent",
    ] {
        assert!(
            names_tool(ORCHESTRATOR_TOML, tool),
            "{tool} is taught but not named"
        );
    }
}

#[test]
fn fingerprint_differs_by_mode_so_a_switch_rebuilds_the_session() {
    let a = fp_with_mode(Some(ThreadMode::Chat));
    let b = fp_with_mode(Some(ThreadMode::Orchestration));
    assert_ne!(a, b);
    assert_eq!(a, fp_with_mode(Some(ThreadMode::Chat)));
}

fn fp_with_mode(
    mode: Option<ThreadMode>,
) -> crate::neppy::web_chat::types::SessionCacheFingerprint {
    crate::neppy::web_chat::types::SessionCacheFingerprint {
        model_override: None,
        temperature: None,
        controls: Default::default(),
        target_agent_id: "orchestrator".into(),
        provider_binding: "p".into(),
        autonomy_signature: "a".into(),
        model_registry_signature: "m".into(),
        profile_signature: "pr".into(),
        mode,
        turn_workspace: None,
        debug_prompt_hash: None,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_mode_session_hides_spawning_tools_but_keeps_mcp_and_memory_helpers() {
    let chat = visible_for(ThreadMode::Chat);
    let orchestration = visible_for(ThreadMode::Orchestration);

    // Hidden: every fleet tool.
    for hidden in CHAT_HIDDEN_TOOLS {
        assert!(
            !chat.contains(*hidden),
            "chat-mode agent must not see `{hidden}`; visible={chat:?}"
        );
    }
    // The only difference between the two belts is the fleet tools: chat loses
    // nothing else (some groups, such as the MCP registry tools, are packed and
    // reachable through load_skill in both modes, so compare rather than
    // assume which names are directly visible).
    let lost: HashSet<&String> = orchestration.difference(&chat).collect();
    for name in &lost {
        assert!(
            CHAT_HIDDEN_TOOLS.contains(&name.as_str()),
            "chat mode lost `{name}`, which is not a fleet tool"
        );
    }
    assert!(
        chat.difference(&orchestration).next().is_none(),
        "chat must add nothing"
    );

    // Kept: direct memory, the resume path, and the synthesised capability
    // helpers (MCP, memory retrieval).
    for kept in ["continue_subagent", "memory_recall", "memory_store"] {
        assert!(
            chat.contains(kept),
            "chat-mode agent lost `{kept}`; visible={chat:?}"
        );
    }
    // The MCP helper (`use_mcp_server`) lives in the `integrations` tool pack:
    // withheld from the schema in BOTH modes and reached through
    // `load_skill` / `use_skill`. Chat mode must keep that route open and must
    // not revoke the pack tools (hide_tools only revokes what it names).
    assert!(
        chat.contains("load_skill") && chat.contains("use_skill"),
        "chat mode must keep the pack route to the MCP helper; visible={chat:?}"
    );
    assert!(
        chat.contains("retrieve_memory") || chat.contains("delegate_retrieve_memory"),
        "chat mode must keep the memory helper; visible={chat:?}"
    );
    for helper in ["use_mcp_server", "setup_mcp_server", "mcp_registry_status"] {
        assert!(
            !CHAT_HIDDEN_TOOLS.contains(&helper),
            "{helper} must not be hidden by chat mode"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn orchestration_mode_session_exposes_close_steer_wait_and_the_spawns() {
    let visible = visible_for(ThreadMode::Orchestration);
    for tool in [
        "spawn_async_subagent",
        "spawn_parallel_agents",
        "list_subagents",
        "steer_subagent",
        "close_subagent",
        "wait_subagent",
        "continue_subagent",
    ] {
        assert!(
            visible.contains(tool),
            "orchestration-mode agent must see `{tool}`; visible={visible:?}"
        );
    }
    assert!(!visible.contains("wait"), "`wait` stays retired");
}

/// W1: an orchestrator session built the way a non-web caller builds it (CLI,
/// medulla, task-board / Pet hand-off — `Agent::from_config_for_agent`, no
/// thread mode declared) gets chat semantics at turn time: the fleet tools are
/// hidden and the turn runs under Chat mode so the `delegate_*` helpers run
/// blocking (`spawn_subagent` stays available; see the seam's docs).
/// A turn that declares Orchestration keeps the full supervisor belt.
#[tokio::test(flavor = "multi_thread")]
async fn non_web_orchestrator_turn_without_a_mode_hides_the_fleet_tools() {
    AgentDefinitionRegistry::init_global_builtins().expect("builtins");
    let workspace = tempfile::TempDir::new().expect("tempdir");
    let config = crate::neppy::config::Config {
        workspace_dir: workspace.path().to_path_buf(),
        action_dir: workspace.path().to_path_buf(),
        ..crate::neppy::config::Config::default()
    };
    let visible = |agent: &crate::neppy::agent::Agent| -> HashSet<String> {
        let v = agent.visible_tool_names_for_test();
        if v.is_empty() {
            agent.tool_specs().iter().map(|s| s.name.clone()).collect()
        } else {
            v.clone()
        }
    };

    let mut cli = crate::neppy::agent::Agent::from_config_for_agent(&config, MODE_AWARE_AGENT_ID)
        .expect("build orchestrator");
    assert!(
        ORCHESTRATION_ONLY_TOOLS
            .iter()
            .any(|t| visible(&cli).contains(*t)),
        "precondition: the definition names the fleet tools"
    );
    assert!(cli.apply_turn_mode_tool_surface(None), "no mode → chat");
    let after = visible(&cli);
    for hidden in CHAT_HIDDEN_TOOLS {
        assert!(!after.contains(*hidden), "`{hidden}` must be hidden");
    }
    assert!(after.contains("continue_subagent"));
    // Idempotent on the next turn.
    assert!(cli.apply_turn_mode_tool_surface(None));
    assert_eq!(visible(&cli), after);

    let mut orch = crate::neppy::agent::Agent::from_config_for_agent(&config, MODE_AWARE_AGENT_ID)
        .expect("build orchestrator");
    let before = visible(&orch);
    assert!(!orch.apply_turn_mode_tool_surface(Some(ThreadMode::Orchestration)));
    assert_eq!(visible(&orch), before, "orchestration keeps the belt");
    for tool in ORCHESTRATION_ONLY_TOOLS {
        assert!(visible(&orch).contains(*tool), "{tool} kept");
    }
}

#[test]
fn debug_thread_routes_to_debug_agent_whatever_the_profile_says() {
    for profile_agent in ["orchestrator", "researcher", "code_executor"] {
        assert_eq!(
            target_agent_id(profile_agent.to_string(), ThreadMode::Debug),
            DEBUG_AGENT_ID
        );
    }
    // Non-debug threads keep the profile's agent untouched.
    for mode in [ThreadMode::Chat, ThreadMode::Orchestration] {
        assert_eq!(
            target_agent_id("orchestrator".to_string(), mode),
            "orchestrator"
        );
        assert_eq!(
            target_agent_id("researcher".to_string(), mode),
            "researcher"
        );
    }
}

#[test]
fn debug_agent_runs_in_debug_mode_only() {
    assert_eq!(
        effective_mode(DEBUG_AGENT_ID, ThreadMode::Debug),
        Some(ThreadMode::Debug)
    );
    assert_eq!(effective_mode(DEBUG_AGENT_ID, ThreadMode::Chat), None);
    // The orchestrator never reports Debug on its own.
    assert_eq!(
        effective_mode("orchestrator", ThreadMode::Chat),
        Some(ThreadMode::Chat)
    );
    assert!(debug_agent_allowed(DEBUG_AGENT_ID, ThreadMode::Debug));
    assert!(!debug_agent_allowed(DEBUG_AGENT_ID, ThreadMode::Chat));
    assert!(!debug_agent_allowed(
        DEBUG_AGENT_ID,
        ThreadMode::Orchestration
    ));
    assert!(debug_agent_allowed("orchestrator", ThreadMode::Chat));
}

#[test]
fn debug_addendum_states_the_ground_rules() {
    let debug = prompt_addendum(ThreadMode::Debug);
    assert!(debug.contains("Debug mode"));
    assert!(debug.contains("debug_report"));
    assert!(debug.contains("debug_run_check"));
    assert!(debug.contains("git push"));
    assert!(!debug.to_lowercase().contains("openhuman"));
}

#[test]
fn fingerprint_differs_by_turn_workspace_so_a_moved_root_rebuilds_the_session() {
    let a = fp_with_mode(Some(ThreadMode::Debug));
    let mut b = fp_with_mode(Some(ThreadMode::Debug));
    assert_eq!(a, b);
    b.turn_workspace = Some(std::path::PathBuf::from("/some/other/checkout"));
    assert_ne!(a, b);
}

#[tokio::test]
async fn debug_prompt_hash_tracks_debug_settings_and_is_none_outside_debug() {
    use super::super::session::debug_prompt_hash;
    use crate::neppy::agent::debug_mode::ops::DebugCtx;
    use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
    use crate::neppy::config::schema::debug_mode::DebugModeConfig;

    let turn = |settings: DebugModeConfig| DebugTurn {
        ctx: DebugCtx::new(&std::env::temp_dir()),
        root: std::env::temp_dir(),
        task_id: None,
        checkpoint_id: None,
        settings,
    };
    let base = DebugModeConfig::default();
    let changed = DebugModeConfig {
        allow_git_push: !base.allow_git_push,
        max_repair_iterations: base.max_repair_iterations + 3,
        ..base.clone()
    };
    let h_base = with_turn(turn(base.clone()), async {
        debug_prompt_hash(Some(ThreadMode::Debug))
    })
    .await;
    let h_same = with_turn(turn(base), async {
        debug_prompt_hash(Some(ThreadMode::Debug))
    })
    .await;
    let h_changed = with_turn(turn(changed), async {
        debug_prompt_hash(Some(ThreadMode::Debug))
    })
    .await;
    assert!(h_base.is_some());
    assert_eq!(h_base, h_same, "same settings, same fingerprint");
    assert_ne!(h_base, h_changed, "edited Debug settings must rebuild");
    for m in [
        None,
        Some(ThreadMode::Chat),
        Some(ThreadMode::Orchestration),
    ] {
        assert_eq!(debug_prompt_hash(m), None);
    }
}
