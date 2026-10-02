//! Trust plumbing for the Pet desktop companion (T2b): the `pet_companion`
//! lane definition, its memory-write ban, the Pet-inbox surfacing of
//! `PetCompanion` parks, and where the `PetCompanion` origin may be built.
//! The gate semantics themselves are pinned in
//! `security::approval::gate_pet_companion_tests`.

use std::collections::HashSet;

use super::{agent_forbids_memory_writes, validate_research_definition, PET_COMPANION_AGENT_ID};
use crate::neppy::agent::harness::definition::{
    AgentDefinition, PromptSource, SandboxMode, SubagentEntry, ToolScope, TriggerMemoryAgent,
};
use crate::neppy::agent::turn_origin;
use crate::neppy::pet::surface::{
    filter_background_approvals, is_pet_surfaceable, PET_COMPANION_ORIGIN_CLASS,
};
use crate::neppy::security::approval::PendingApproval;

fn builtin(id: &str) -> AgentDefinition {
    crate::neppy::agent::registry::agents::load_builtins()
        .expect("built-in TOML must parse")
        .into_iter()
        .find(|d| d.id == id)
        .unwrap_or_else(|| panic!("missing built-in {id}"))
}

// ── lane ─────────────────────────────────────────────────────────────────────

#[test]
fn both_pet_lanes_forbid_post_turn_memory_writes() {
    assert!(agent_forbids_memory_writes(PET_COMPANION_AGENT_ID));
    assert!(agent_forbids_memory_writes("pet_research"));
    assert!(!agent_forbids_memory_writes("orchestrator"));
    assert!(!agent_forbids_memory_writes("pet_companion_x"));
}

#[test]
fn the_built_in_companion_definition_is_accepted() {
    let def = builtin(PET_COMPANION_AGENT_ID);
    assert_eq!(validate_research_definition(&def), Ok(()));
    // The research lane is still validated by its own rules.
    assert_eq!(
        validate_research_definition(&builtin("pet_research")),
        Ok(())
    );
}

#[test]
fn loosened_companion_definitions_are_refused() {
    let shipped = builtin(PET_COMPANION_AGENT_ID);
    let variant = |f: &dyn Fn(&mut AgentDefinition)| {
        let mut d = shipped.clone();
        f(&mut d);
        d
    };
    let cases: Vec<(&str, AgentDefinition)> = vec![
        (
            "a tool",
            variant(&|d| d.tools = ToolScope::Named(vec!["memory_recall".into()])),
        ),
        (
            "wildcard scope",
            variant(&|d| d.tools = ToolScope::Wildcard),
        ),
        (
            "extra_tools",
            variant(&|d| d.extra_tools = vec!["web_fetch".into()]),
        ),
        (
            "TOML prompt override",
            variant(&|d| d.system_prompt = PromptSource::Inline("be helpful".into())),
        ),
        (
            "sandbox none",
            variant(&|d| d.sandbox_mode = SandboxMode::None),
        ),
        (
            "memory agent trigger",
            variant(&|d| d.trigger_memory_agent = TriggerMemoryAgent::Always),
        ),
        (
            "omitted safety preamble",
            variant(&|d| d.omit_safety_preamble = true),
        ),
        (
            "subagent",
            variant(&|d| d.subagents = vec![SubagentEntry::AgentId("researcher".into())]),
        ),
    ];
    for (label, def) in cases {
        assert!(
            validate_research_definition(&def).is_err(),
            "{label} must be refused"
        );
    }
}

/// A research-shaped definition under the companion id is refused (it has
/// tools), so a workspace file cannot swap the lanes.
#[test]
fn the_research_definition_cannot_pose_as_the_companion() {
    let mut def = builtin("pet_research");
    def.id = PET_COMPANION_AGENT_ID.to_string();
    assert!(validate_research_definition(&def).is_err());
}

// ── surfacing ────────────────────────────────────────────────────────────────

fn row(id: &str, class: &str) -> PendingApproval {
    PendingApproval::new(
        id,
        "channels.proactive_send",
        "send",
        serde_json::json!({}),
        None,
    )
    .with_origin_class(class)
}

#[test]
fn the_companion_class_is_the_origin_class() {
    assert_eq!(
        turn_origin::pet_companion_origin("pet-companion:x", Some("t".into())).class(),
        PET_COMPANION_ORIGIN_CLASS
    );
}

#[test]
fn companion_parks_are_pet_surfaceable() {
    assert!(is_pet_surfaceable(&row("c", PET_COMPANION_ORIGIN_CLASS)));
    // A flow context still has its own surface.
    let with_flow = row("f", PET_COMPANION_ORIGIN_CLASS).with_source_context(
        crate::neppy::security::approval::ApprovalSourceContext::Flow {
            flow_id: "f".into(),
            run_id: "r".into(),
            node_id: None,
        },
    );
    assert!(!is_pet_surfaceable(&with_flow));
}

/// A companion park routed as a card on its hand-off thread is STILL listed in
/// the Pet inbox; a chat-routed WebChat park is not (its card is the surface).
#[test]
fn chat_routed_companion_parks_stay_in_the_inbox() {
    let rows = vec![
        row("companion-routed", PET_COMPANION_ORIGIN_CLASS),
        row("companion-headless", PET_COMPANION_ORIGIN_CLASS),
        row("webchat-routed", "WebChat"),
        row("webchat-orphan", "WebChat"),
        row("background", "TrustedAutomation(BackgroundTurn)"),
    ];
    let routed: HashSet<String> = ["companion-routed", "webchat-routed"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let kept: HashSet<String> = filter_background_approvals(rows, &routed)
        .into_iter()
        .map(|r| r.request_id)
        .collect();
    let want: HashSet<String> = ["companion-routed", "companion-headless", "webchat-orphan"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(kept, want);
}

// ── construction sites ───────────────────────────────────────────────────────

/// `TrustedAutomationSource::PetCompanion` (and its constructor) appear only
/// where the origin is defined, gated, or legitimately built: the companion
/// runtime (`pet/companion/runtime/`, T2c) and the hand-off executor. A new
/// site must be added here deliberately, never by accident.
#[test]
fn the_companion_origin_is_built_only_by_the_companion_and_its_executor() {
    const ALLOWED_FILES: &[&str] = &[
        "src/neppy/agent/turn_origin.rs",
        "src/neppy/security/approval/gate.rs",
        "src/neppy/security/approval/gate_pet_companion_tests.rs",
        "src/neppy/agent/task_dispatcher/mod.rs",
        "src/neppy/agent/task_dispatcher/origin_tests.rs",
        "src/neppy/pet/companion_trust_tests.rs",
    ];
    const ALLOWED_DIRS: &[&str] = &["src/neppy/pet/companion/runtime/"];
    const NEEDLES: &[&str] = &[
        "TrustedAutomationSource::PetCompanion",
        "PetCompanion {",
        "pet_companion_origin(",
    ];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut stack = vec![root.join("src")];
    let mut offenders = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if ALLOWED_FILES.contains(&rel.as_str())
                || ALLOWED_DIRS.iter().any(|d| rel.starts_with(d))
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if NEEDLES.iter().any(|n| text.contains(n)) {
                offenders.push(rel);
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "the PetCompanion origin is referenced outside its allowed sites: {offenders:?}"
    );
}
