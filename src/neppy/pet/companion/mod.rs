//! Pet desktop companion: the pure, testable core. No sensors, no I/O beyond
//! `companion.db`, no model calls (the runtime ticket builds those on top).
//!
//! Pipeline: observation -> [`sensitive::scrub`] -> [`exclusions`] ->
//! [`buffer`] -> [`activity`] -> [`usefulness`] -> [`ratelimit`] ->
//! [`policy`] -> suggest / execute -> [`store`] (scrubbed excerpts only).
//!
//! Privacy invariants enforced by types: `sensitive::Scrubbed` is the only text
//! type an `ObservationEvent` accepts and only `scrub` can build one;
//! `ObservationEvent` has no `Serialize`; high-risk categories are never
//! executed by the companion (`policy::decide` returns `Refuse`).

pub mod activity;
pub mod buffer;
pub mod exclusions;
pub mod policy;
pub mod ratelimit;
pub mod sensitive;
pub mod settings;
pub mod store;
pub mod types;
pub mod usefulness;
