//! Debug Mode (Phase 1) — the core domain behind the `debug_mode` RPC namespace.
//!
//! Gives an in-app debugging session a safe substrate to work on the project
//! source: PROJECT_ROOT resolution, git status, non-destructive checkpoints
//! pinned under `refs/neppy-debug/`, a reversible rollback, diffs, task
//! history, discovered validation checks, a guarded `run_check`, and an audit
//! log. No UI and no LLM calls live here.
//!
//! Phase 2 adds the agent seam: [`turn`] (repo scope + automatic task and
//! checkpoint per Debug-mode chat turn) and [`tools`] (`debug_checkpoint`,
//! `debug_run_check`, `debug_report`).
//!
//! Storage: `{workspace}/debug_mode/{history.json,checkpoints.json,audit.jsonl}`
//! (agent tools cannot write the workspace dir, so history is tamper-resistant
//! from the debug agent itself).

pub mod budget;
pub mod candidate;
mod candidate_schemas;
mod candidate_steps;
mod checkpoints;
pub mod checks;
mod commit;
mod exec;
mod git;
pub mod local_install;
mod local_install_schemas;
mod local_install_steps;
pub mod ops;
mod pass_gate;
pub mod policy;
mod policy_scope;
#[cfg(test)]
#[path = "policy_scope_tests.rs"]
mod policy_scope_tests;
pub mod release;
mod release_preflight;
mod release_schemas;
mod release_steps;
mod schema_defs;
mod schemas;
pub mod secrets;
pub mod selfmod;
pub mod settings;
mod shell_parse;
mod store;
#[cfg(test)]
mod test_util;
pub mod tools;
mod tools_check;
pub mod turn;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_debug_mode_controller_schemas,
    all_registered_controllers as all_debug_mode_registered_controllers,
};
