//! Pet mode — a background, read-only research companion.
//!
//! While the Mac is awake, a cron agent job (`pet:<id>:research`, agent
//! `pet_research`) scans the user's enabled sources and records private notes
//! through the `pet_note` tool. After each pass a deterministic surfacer ranks
//! the notes: a few interrupt the user (bounded by a daily budget and quiet
//! hours), the rest wait for one ranked daily digest. Nothing is ever sent:
//! the lane runs under `TrustedAutomationSource::PetResearch`, which the
//! approval gate denies for every external effect, behind a closed tool
//! allowlist and a `read_only` sandbox. Suggested actions become proposals the
//! user accepts into a chat composer.
//!
//! Housed under `DomainGroup::Automation` (it cannot run without the cron
//! scheduler). Storage: `{workspace}/pet/pet.db`.

pub mod bus;
pub mod companion;
mod digest;
mod lane;
pub mod ops;
mod run;
mod schemas;
mod store;
mod store_feed;
mod store_notes;
mod surface;
mod surfacer;
pub mod tools;
pub mod types;

pub use lane::{agent_forbids_memory_writes, validate_research_definition};
pub use schemas::{
    all_controller_schemas as all_pet_controller_schemas,
    all_registered_controllers as all_pet_registered_controllers,
};
pub use surface::surface_after_pass;

pub use types::{PET_JOB_NAME_PREFIX, PET_RESEARCH_AGENT_ID, PET_RESEARCH_TOOL_ALLOWLIST};

/// Whether a `ProactiveMessageRequested` is the pet digest. The pet never
/// sends: that message belongs to the in-app thread only and must not be
/// echoed to an external channel (Telegram, Discord, ...), whatever the
/// user's approval settings are.
pub fn is_pet_proactive_message(source: &str, job_name: Option<&str>) -> bool {
    job_name == Some(types::PET_DIGEST_JOB_NAME)
        || source.starts_with(types::PET_PROACTIVE_SOURCE_PREFIX)
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod bus_tests;
#[cfg(test)]
#[path = "digest_tests.rs"]
mod digest_tests;
#[cfg(test)]
#[path = "ops_tests.rs"]
mod ops_tests;
#[cfg(test)]
#[path = "run_tests.rs"]
mod run_tests;
#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
#[cfg(test)]
#[path = "surfacer_tests.rs"]
mod surfacer_tests;
#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;
