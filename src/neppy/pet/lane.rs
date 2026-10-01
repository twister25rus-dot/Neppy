//! Invariants of the Pet research lane that the session builder enforces.
//!
//! The lane reads untrusted content unattended, so its safety rests on its
//! agent definition (closed tool allowlist, read-only sandbox, no delegation).
//! A workspace `agents/pet_research.toml` can replace the built-in definition
//! (custom definitions override built-ins on id collision), so the builder
//! re-checks the resolved definition and refuses to build the lane otherwise.

use crate::neppy::agent::harness::definition::{AgentDefinition, SandboxMode, ToolScope};

use super::types::{PET_RESEARCH_AGENT_ID, PET_RESEARCH_TOOL_ALLOWLIST};

/// Whether `agent_id` is the Pet research lane, whose sessions must install no
/// post-turn memory writer (auto-save, archivist capture, learning hooks): the
/// lane writes only pet notes, never the user's main memory.
pub fn agent_forbids_memory_writes(agent_id: &str) -> bool {
    agent_id == PET_RESEARCH_AGENT_ID
}

/// `Ok` only when `def` is exactly the read-only research lane: sandbox
/// `read_only`, tool scope `Named` equal (as a set) to
/// [`PET_RESEARCH_TOOL_ALLOWLIST`], no extra tools, no skill filter and no
/// subagents. The error text is for logs and the run record.
pub fn validate_research_definition(def: &AgentDefinition) -> Result<(), String> {
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
    if !def.extra_tools.is_empty() {
        return Err("extra_tools must be empty".into());
    }
    if def.skill_filter.is_some() {
        return Err("skill_filter must be unset".into());
    }
    if !def.subagents.is_empty() {
        return Err("the pet research lane may not delegate to subagents".into());
    }
    Ok(())
}
