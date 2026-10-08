//! System prompt builder for the `debug_agent` built-in agent (Debug mode).
//!
//! Returns the fully-assembled system prompt, including the standard
//! `## Safety` block (this agent has `omit_safety_preamble = false`
//! in its TOML — it runs commands and edits source, so the guard rails
//! stay inlined).

use crate::neppy::agent::context::prompt::{
    render_safety, render_tools, render_user_files, render_workspace, PromptContext,
};
use crate::neppy::agent::prompts::WORK_METHOD_BODY;
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    // Shared method block (how to investigate, plan, verify, report). Spliced
    // in here, once, rather than by the central builder: sub-agents keep their
    // narrow prompts.
    out.push_str(WORK_METHOD_BODY);
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

    let safety = render_safety();
    out.push_str(safety.trim_end());
    out.push_str("\n\n");

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::agent::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    #[test]
    fn build_returns_nonempty_body() {
        let visible: HashSet<String> = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "debug_agent",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &visible,
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
        };
        let body = build(&ctx).unwrap();
        assert!(!body.is_empty());
    }

    /// Throwaway workspace: the central builder seeds STYLE.md into it.
    fn scratch_workspace() -> &'static std::path::Path {
        use std::sync::OnceLock;
        static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
        DIR.get_or_init(|| {
            let dir = tempfile::TempDir::new().expect("temp workspace");
            let path = dir.path().to_path_buf();
            std::mem::forget(dir);
            path
        })
        .as_path()
    }

    fn built_debug_prompt() -> String {
        use crate::neppy::agent::prompts::SystemPromptBuilder;
        let visible: HashSet<String> = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: scratch_workspace(),
            model_name: "test",
            agent_id: "debug_agent",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &visible,
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
        };
        // Same path the session uses: the central builder wrapping `build`.
        SystemPromptBuilder::from_dynamic(build)
            .build(&ctx)
            .unwrap()
    }

    #[test]
    fn built_prompt_carries_the_work_method_exactly_once() {
        let body = built_debug_prompt();
        assert_eq!(body.matches("## How you work").count(), 1);
        assert!(body.contains("Verify before you claim success"));
        assert!(body.contains("Never edit a file you have not read"));
        let method = body.find("## How you work").unwrap();
        let finishing = body.find("## Finishing").unwrap();
        assert!(finishing < method, "method follows the role text");
        assert!(
            body.contains("## Debug rules (mandatory)"),
            "debug rules section present"
        );
    }

    #[test]
    fn working_style_allows_rerunning_a_check_after_an_edit() {
        let body = built_debug_prompt();
        assert!(body.contains("never repeat a call with identical arguments** unless something changed since the last call"));
        assert!(body.contains("re-running a check after a fix"));
    }

    #[test]
    fn prompt_names_debug_run_check_as_the_way_to_verify() {
        let body = built_debug_prompt();
        assert!(body.contains("`debug_run_check`"));
        assert!(
            ARCHETYPE.contains("only `debug_run_check` results count"),
            "the TEST stage must point at the tool the pass gate reads"
        );
        assert!(ARCHETYPE.contains("records any other `pass` as `partial`"));
    }

    #[test]
    fn prompt_routes_packed_run_workflow_through_use_skill() {
        use crate::neppy::tools::toolpacks::registry;
        let pack = registry::pack_for_tool("run_workflow").expect("`run_workflow` is packed");
        assert_eq!(pack.id, "workflows", "the prompt names pack `workflows`");
        assert!(ARCHETYPE.contains(
            "`run_workflow` is packed, not missing: call `use_skill` with skill `workflows` and tool `run_workflow`"
        ));
    }

    #[test]
    fn working_style_decides_the_next_step_in_reasoning_not_the_reply() {
        assert!(ARCHETYPE.contains("Decide your next step in your reasoning, not in your reply"));
        assert!(!ARCHETYPE.contains("State your next step in a sentence"));
    }

    #[test]
    fn prompt_md_has_no_em_dash() {
        assert!(!ARCHETYPE.contains('\u{2014}'), "STYLE bans em-dashes");
    }
}
