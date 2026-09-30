//! Fault injection for the resume tests.
//!
//! The step machine asks [`Faults::crash_at`] after each durable transition.
//! Production uses [`NoFaults`]; a test answers `true` at one point to simulate
//! the process dying right there, then runs the task again against the same
//! store to prove nothing is repeated.

use super::types::{AssistantError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// The plan is stored; no side effect has happened.
    Planned,
    /// Edit `n`'s intent is recorded; the file is untouched.
    Intent(usize),
    /// Edit `n`'s file is written; the ledger still says `intent`.
    Write(usize),
    /// Every edit is applied; the step is not yet `applied`.
    Applied,
    /// The test's intent is recorded; it has not run.
    TestIntent,
    /// The step is `tested`; the task record is not yet updated.
    Tested,
}

pub trait Faults: Send + Sync {
    fn crash_at(&self, point: FaultPoint) -> bool;
}

pub struct NoFaults;

impl Faults for NoFaults {
    fn crash_at(&self, _point: FaultPoint) -> bool {
        false
    }
}

/// `Err` when the injector says the process dies at `point`.
pub(crate) fn check(faults: &dyn Faults, point: FaultPoint) -> Result<()> {
    if faults.crash_at(point) {
        log::warn!("[local_assistant:fault] simulated crash at {point:?}");
        return Err(AssistantError::Interrupted(format!(
            "fault injected at {point:?}"
        )));
    }
    Ok(())
}
