//! Invariants of the Pet's agent lanes that the session builder enforces.
//!
//! Two lanes, both read-only:
//!
//! * `pet_research` reads untrusted content unattended, so its safety rests on
//!   its agent definition (closed tool allowlist, read-only sandbox, no
//!   delegation).
//! * `pet_companion` writes suggestion text from scrubbed desktop excerpts. It
//!   is **tool-less** (an empty `named` scope), read-only and never delegates;
//!   anything that must act is a hand-off run under the `PetCompanion` origin.
//!
//! A workspace `agents/<id>.toml` can replace a built-in definition (custom
//! definitions override built-ins on id collision), so the builder re-checks
//! the resolved definition and refuses to build the lane otherwise. Neither
//! lane installs a post-turn memory writer.

use crate::neppy::agent::harness::definition::{
    AgentDefinition, PromptSource, SandboxMode, ToolScope, TriggerMemoryAgent,
};

use super::types::{PET_RESEARCH_AGENT_ID, PET_RESEARCH_TOOL_ALLOWLIST};

/// Agent id of the Pet desktop companion's suggestion lane (zero tools).
pub(crate) const PET_COMPANION_AGENT_ID: &str = "pet_companion";

/// Whether `agent_id` is a Pet lane (`pet_research` or `pet_companion`), whose
/// sessions must install no post-turn memory writer (auto-save, archivist
/// capture, learning hooks): the research lane writes only pet notes and the
/// companion writes nothing, never the user's main memory.
pub fn agent_forbids_memory_writes(agent_id: &str) -> bool {
    agent_id == PET_RESEARCH_AGENT_ID || agent_id == PET_COMPANION_AGENT_ID
}

/// `Ok` only when `def` is exactly the built-in Pet lane for its id: the
/// companion rules for `pet_companion` (see [`validate_companion_definition`]),
/// the research rules otherwise. The error text is for logs and the run record.
///
/// Research rules: sandbox `read_only`, tool scope `Named` equal (as a set) to
/// [`PET_RESEARCH_TOOL_ALLOWLIST`], no extra tools, no skill filter, no
/// subagents, `trigger_memory_agent = never`, the safety preamble kept, and the
/// built-in (function-built) prompt.
pub fn validate_research_definition(def: &AgentDefinition) -> Result<(), String> {
    if def.id == PET_COMPANION_AGENT_ID {
        return validate_companion_definition(def);
    }
    if def.sandbox_mode != SandboxMode::ReadOnly {
        return Err(format!(
            "sandbox_mode is {:?}, expected read_only",
            def.sandbox_mode
        ));
    }
    let ToolScope::Named(names) = &def.tools else {
        return Err("tool scope is not a closed `named` allowlist".into());
    };
    let have: std::collections::BTreeSet<&str> = names.iter().map(String::as_str).collect();
    let want: std::collections::BTreeSet<&str> =
        PET_RESEARCH_TOOL_ALLOWLIST.iter().copied().collect();
    if have != want {
        return Err("tool allowlist differs from the pet research allowlist".into());
    }
    validate_shared_lane_rules(def, "pet research")
}

/// Companion rules: sandbox `read_only`, tool scope `named = []` (no tools at
/// all, no wildcard), plus every rule the research lane has (no extra tools, no
/// skill filter, `trigger_memory_agent = never`, safety preamble kept, the
/// built-in prompt, no subagents).
fn validate_companion_definition(def: &AgentDefinition) -> Result<(), String> {
    if def.sandbox_mode != SandboxMode::ReadOnly {
        return Err(format!(
            "sandbox_mode is {:?}, expected read_only",
            def.sandbox_mode
        ));
    }
    match &def.tools {
        ToolScope::Named(names) if names.is_empty() => {}
        ToolScope::Named(_) => return Err("the pet companion lane must have no tools".into()),
        ToolScope::Wildcard => {
            return Err("tool scope is a wildcard; the pet companion must be `named = []`".into())
        }
    }
    validate_shared_lane_rules(def, "pet companion")
}

/// Rules both lanes share, checked after the tool scope. `lane` names the lane
/// in the error text.
fn validate_shared_lane_rules(def: &AgentDefinition, lane: &str) -> Result<(), String> {
    if !def.extra_tools.is_empty() {
        return Err("extra_tools must be empty".into());
    }
    if def.skill_filter.is_some() {
        return Err("skill_filter must be unset".into());
    }
    if def.trigger_memory_agent != TriggerMemoryAgent::Never {
        return Err("trigger_memory_agent must be never (no memory agent before the pass)".into());
    }
    if def.omit_safety_preamble {
        return Err("the safety preamble may not be omitted".into());
    }
    // Only the built-in definition carries a function-built prompt (a TOML
    // override can only be `inline` or `file`), so this pins the shipped prompt.
    if !matches!(def.system_prompt, PromptSource::Dynamic(_)) {
        return Err(format!("system prompt is not the built-in {lane} prompt"));
    }
    if !def.subagents.is_empty() {
        return Err(format!("the {lane} lane may not delegate to subagents"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "companion_trust_tests.rs"]
mod companion_trust_tests;
