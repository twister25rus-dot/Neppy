//! Clock and sampler cadence (injectable for tests).

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

/// Wall and monotonic clock (injectable for tests).
pub trait Clock: Send + Sync {
    fn instant(&self) -> Instant;
    fn utc(&self) -> DateTime<Utc>;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn instant(&self) -> Instant {
        Instant::now()
    }
    fn utc(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Sampler cadence.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Between samples while the user is active.
    pub active: Duration,
    /// Between samples while the user is idle (>60 s).
    pub idle: Duration,
    /// Park while the gate is closed (woken early on enable / resume / lease).
    pub parked: Duration,
    /// Floor between two autonomous captures, even on window change.
    pub capture_floor: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            active: Duration::from_secs(2),
            idle: Duration::from_secs(10),
            parked: Duration::from_secs(5),
            capture_floor: Duration::from_secs(3),
        }
    }
}
