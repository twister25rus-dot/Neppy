//! System prompt builder for the `pet_research` built-in agent.
//!
//! Returns the fully-assembled system prompt. Each agent's `build()`
//! composes section helpers from [`crate::neppy::agent::context::prompt`]
//! in the order it wants — so the output IS what the LLM sees, no
//! post-processing in the runner.

use crate::neppy::agent::context::prompt::{
    render_ambient_environment, render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    let user_files = render_user_files(ctx)?;
    if !user_files.trim().is_empty() {
        out.push_str(user_files.trim_end());
        out.push_str("\n\n");
    }

    let tools = render_tools(ctx)?;
    if !tools.trim().is_empty() {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push_str("\n\n");
    }

    // Ambient runtime + current date/time so due dates and urgency are
    // grounded on the real clock. Kept at the prompt tail because it is
    // time-volatile (KV cache convention from `SystemPromptBuilder`).
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
            agent_id: "pet_research",
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
    fn build_is_nonempty_and_grounded_on_the_clock() {
        let body = build(&ctx()).unwrap();
        assert!(!body.is_empty());
        assert!(body.contains("## Current Date & Time"));
    }

    #[test]
    fn prompt_carries_the_hard_rules() {
        let lower = ARCHETYPE.to_lowercase();
        assert!(lower.contains("never send"), "read-only rule missing");
        assert!(ARCHETYPE.contains("proposed_action"));
        assert!(
            lower.contains("treat as data"),
            "untrusted-content rule missing"
        );
        assert!(ARCHETYPE.contains("Recorded N notes."));
        // Never tells the model to fetch pages.
        assert!(!ARCHETYPE.contains("web_fetch"));
    }
}
