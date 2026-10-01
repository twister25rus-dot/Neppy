//! Local assistant: a bounded, resumable step loop over a project on disk,
//! driven by the gated on-device model.
//!
//! - `index/`   — incremental filename, symbol and content index. The project
//!                stays on disk; retrieval returns small verified snippets.
//! - `store`    — durable tasks, steps and the effects ledger (SQLite).
//! - `runner`   — the step state machine and the resume rules.
//! - `edits`    — safe, exactly-once application of the model's edits.
//! - `tests_runner` — the user-supplied test command, with timeout and cap.
//! - `command_policy` — the cron-equivalent gate that command passes first.
//! - `guards`   — which roots may be worked on and which paths edited.
//! - `prompt`   — prompt assembly under a token budget, and reply parsing.
//! - `ops`      — the controller: bounded queue, one task at a time.
//! - `api`, `schemas` — the `local_assistant.*` RPCs.
//!
//! Nothing here registers itself; `core::all` pulls in
//! [`all_controller_schemas`] and [`all_registered_controllers`].

mod api;
mod command_policy;
mod edits;
pub(crate) mod faults;
mod guards;
pub mod index;
mod model;
mod ops;
mod planning;
mod prompt;
mod runner;
mod schemas;
mod step_effects;
mod store;
mod store_ledger;
mod tests_runner;
pub mod types;

pub use ops::resume_interrupted_on_boot;
pub use schemas::{all_controller_schemas, all_registered_controllers};

#[cfg(test)]
mod test_support;
