//! Wiring the pack tools into an agent's registry and visible set.

use std::collections::HashSet;
use std::sync::{Arc, Weak};

use super::registry;
use super::tools::{LoadSkillTool, PackRegistryHandle, UseSkillTool, LOAD_SKILL, USE_SKILL};
use crate::neppy::tools::traits::Tool;

/// Append `load_skill` + `use_skill` to a freshly built registry.
///
/// They start unbound; [`bind_pack_registry`] gives them their view of the
/// registry once it is behind an `Arc`.
pub fn append_pack_tools(tools: &mut Vec<Box<dyn Tool>>) {
    let handle = PackRegistryHandle::default();
    tools.push(Box::new(LoadSkillTool::new(handle.clone())));
    tools.push(Box::new(UseSkillTool::new(handle)));
}

/// Point the pack tools at the registry they live in, scoped to `reach`.
///
/// `reach` is what [`strip_packed_from_visible`] returned for this agent: the
/// packed tools withheld from its visible set, which are exactly the ones it
/// was entitled to and can now reach only through `load_skill` / `use_skill`.
/// The registry itself is the full global tool set, so without the reach the
/// pack tools would resolve any packed tool in the build — see
/// [`PackRegistryHandle`]'s `reach` field.
///
/// The handle holds a [`Weak`], so the pack tools referencing the very vector
/// that owns them does not leak. Call this after **every** rebinding of the
/// agent's tool `Arc`; a stale handle degrades to "skill unavailable" rather
/// than dispatching to the wrong registry.
pub fn bind_pack_registry<S: AsRef<str>>(tools: &Arc<Vec<Box<dyn Tool>>>, reach: &[S]) {
    let weak: Weak<Vec<Box<dyn Tool>>> = Arc::downgrade(tools);
    let mut bound = 0usize;
    for handle in pack_handles(tools) {
        handle.bind(weak.clone());
        handle.set_reach(reach.iter().map(|n| n.as_ref().to_string()));
        bound += 1;
    }
    tracing::debug!(
        bound,
        reach = reach.len(),
        "[toolpacks] bound pack tools to live registry"
    );
}

/// Extend the pack tools' reach with names just withheld from this agent.
///
/// Only for the output of a [`strip_packed_from_visible`] call on the same
/// agent — that is what makes the names in-scope. Never pass a caller-supplied
/// list.
pub fn grant_pack_reach<S: AsRef<str>>(tools: &[Box<dyn Tool>], names: &[S]) {
    for handle in pack_handles(tools) {
        handle.grant_reach(names);
    }
}

/// Take names out of the pack tools' reach.
///
/// A withheld tool is already absent from the visible set, so removing it from
/// that set — which is all a later narrowing like `Agent::hide_tools` does —
/// leaves it reachable through `use_skill`. Pair every such narrowing with this.
pub fn revoke_pack_reach<S: AsRef<str>>(tools: &[Box<dyn Tool>], names: &[S]) {
    for handle in pack_handles(tools) {
        handle.revoke_reach(names);
    }
}

fn pack_handles(tools: &[Box<dyn Tool>]) -> impl Iterator<Item = &PackRegistryHandle> {
    tools
        .iter()
        .filter(|tool| matches!(tool.name(), LOAD_SKILL | USE_SKILL))
        .filter_map(|tool| tool.pack_registry_handle())
}

/// Remove packed tool names from an agent's advertised set.
///
/// This is the whole compression: the tools stay registered and executable, but
/// their schemas never reach the provider. An agent that declared none of the
/// packed tools is unaffected, and `load_skill` / `use_skill` are only added
/// when the agent actually lost something to a pack — otherwise every narrow
/// sub-agent would grow two tools that can only report an empty skill.
///
/// `agent_id` selects which packs apply: a pack is skipped for the specialist
/// that owns its family (see [`super::types::ToolPack::owners`]), because
/// withholding a belt from the agent that exists to run it only buys a
/// `load_skill` round trip per turn.
///
/// A caller with an *empty* `visible` set means "everything is visible"
/// (the harness's historical sentinel), so there is nothing to subtract from
/// and the set is left alone.
///
/// Returns the names it withheld. Those — and only those — are what the agent
/// may reach through the pack tools, so the caller must hand them to
/// [`bind_pack_registry`] (or [`grant_pack_reach`] on a later re-strip).
/// Dropping the return value leaves the pack tools unable to reach anything.
#[must_use = "the withheld names are the agent's pack reach; bind or grant them"]
pub fn strip_packed_from_visible(visible: &mut HashSet<String>, agent_id: &str) -> Vec<String> {
    if visible.is_empty() {
        return Vec::new();
    }
    // Groups an embedder marked `Advertised` keep their schemas on the wire;
    // `Off` groups were never registered, so nothing of theirs can be in
    // `visible` to subtract. Only `Withheld` — the default for every group —
    // is actually withheld here.
    let groups = super::groups::current();
    let packed: Vec<String> = registry::packed_tool_names_for_agent(agent_id)
        .into_iter()
        .filter(|name| groups.mode_for_tool(name) == super::groups::GroupMode::Withheld)
        .filter(|name| visible.contains(*name))
        .map(str::to_string)
        .collect();
    if packed.is_empty() {
        return packed;
    }
    for name in &packed {
        visible.remove(name);
    }
    visible.insert(LOAD_SKILL.to_string());
    visible.insert(USE_SKILL.to_string());
    tracing::info!(
        agent = %agent_id,
        hidden = packed.len(),
        "[toolpacks] withheld packed tool schemas; load_skill/use_skill advertised instead"
    );
    packed
}
