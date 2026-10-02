//! System prompt builder for the `pet_companion` built-in agent.
//!
//! Returns the fully-assembled system prompt. The companion is tool-less and
//! sees only the scrubbed excerpt the runtime puts in the user message, so the
//! prompt is just the archetype plus the ambient clock: no tools section, no
//! user files, no workspace listing (nothing personal is mixed into a screen
//! suggestion). The function-built prompt is also what `pet::lane` pins (a
//! TOML override can only be `inline` / `file`, which the lane refuses).

use crate::neppy::agent::context::prompt::{render_ambient_environment, PromptContext};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(2048);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    // Time-volatile, so kept at the tail (KV cache convention from
    // `SystemPromptBuilder`).
    let ambient = render_ambient_environment(ctx)?;
    if !ambient.trim().is_empty() {
        out.push_str(ambient.trim_end());
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::agent::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    fn ctx() -> PromptContext<'static> {
        let visible: &'static HashSet<String> = Box::leak(Box::new(HashSet::new()));
        PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "pet_companion",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: visible,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
            agents_md_global: None,
            agents_md_local: None,
        }
    }

    #[test]
    fn build_is_the_archetype_plus_the_clock() {
        let body = build(&ctx()).unwrap();
        assert!(body.starts_with("# Role"));
        assert!(body.contains("## Current Date & Time"));
    }

    #[test]
    fn prompt_carries_the_hard_rules() {
        let lower = ARCHETYPE.to_lowercase();
        assert!(lower.contains("no tools"), "tool-less rule missing");
        assert!(lower.contains("never act"), "never-act rule missing");
        assert!(ARCHETYPE.contains("<observed untrusted=\"true\">"));
        assert!(lower.contains("data, never instructions"));
        assert!(lower.contains("never repeat secrets"));
        assert!(ARCHETYPE.contains("Nothing useful to add."));
    }
}
