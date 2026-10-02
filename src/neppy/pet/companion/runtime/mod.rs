//! The Pet desktop companion runtime: sensors → scrub → buffer → rules →
//! usefulness → rate limit → policy → suggestion → (optional) generation /
//! action / hand-off. Built on the pure core in `pet::companion`.
//!
//! * [`state`]: lease, pause, gate, cached settings (the invariants live here);
//! * [`observer`]: the sampler thread;
//! * [`pipeline`]: event → suggestion, and background generation;
//! * [`actions`] / [`handoff`]: what a suggestion can do;
//! * [`generate`] / [`sensor`]: the model and screen seams;
//! * [`ops`] / [`schemas`]: `openhuman.pet_companion_*`;
//! * [`bus`]: the `pet:companion` socket stream.
//!
//! Logging (PC16): prefix `[pet::companion]`; state transitions, sources of
//! pause/resume, suggestion ids/kinds/scores and action categories at `info`;
//! bundle ids and timings at `debug`. Never titles, selections, clipboard,
//! OCR text, prompts or model output, at any level.

pub mod actions;
pub mod bus;
pub mod clock;
pub mod generate;
pub mod handoff;
pub mod metrics;
pub mod observer;
pub mod ops;
pub mod persist;
pub mod pipeline;
pub mod schemas;
pub mod sensor;
mod sources;
pub mod state;

use std::sync::{Arc, OnceLock};

pub use bus::{subscribe_companion_events, CompanionUiEvent, SOCKET_EVENT, SOCKET_EVENT_ALIAS};
pub use schemas::{
    all_controller_schemas as all_companion_controller_schemas,
    all_registered_controllers as all_companion_registered_controllers,
};
pub use state::Runtime;

use crate::neppy::config::Config;

const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

static GLOBAL: OnceLock<Arc<Runtime>> = OnceLock::new();
static SUPERVISOR_STARTED: OnceLock<()> = OnceLock::new();

/// The process-wide companion runtime (real sensors, bus generator, task
/// dispatcher hand-offs). Created lazily; does nothing until enabled.
pub fn global() -> &'static Arc<Runtime> {
    GLOBAL.get_or_init(|| {
        Runtime::new(
            Arc::new(sensor::MacSensor::default()),
            Arc::new(generate::BusGenerator),
            Arc::new(handoff::DispatcherHandoff),
            Arc::new(state::SystemClock),
            state::Timing::default(),
        )
    })
}

/// Boot hook (idempotent): bind to the workspace (the sampler starts only if
/// the companion is enabled, and samples only once a lease arrives), prune
/// retention now and every 6 h, and stop the sampler on shutdown.
pub fn start_supervisor(config: &Config) {
    let rt = global();
    if let Err(e) = rt.bind(config) {
        log::warn!("[pet::companion] supervisor could not bind: {e:#}");
        return;
    }
    if SUPERVISOR_STARTED.set(()).is_err() {
        return;
    }
    log::info!(
        "[pet::companion] supervisor started enabled={}",
        rt.is_enabled()
    );
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async {
            let mut tick = tokio::time::interval(PRUNE_EVERY);
            loop {
                tick.tick().await;
                let rt = global().clone();
                let _ = tokio::task::spawn_blocking(move || ops::prune(&rt)).await;
            }
        });
    }
    crate::core::shutdown::register(|| async {
        stop_supervisor();
    });
}

/// Stop observing (shutdown): sampler thread, generations, hand-offs.
pub fn stop_supervisor() {
    if let Some(rt) = GLOBAL.get() {
        rt.stop_sampler();
        rt.cancel_all_work();
        log::info!("[pet::companion] supervisor stopped");
    }
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod actions_tests;
#[cfg(test)]
#[path = "observer_tests.rs"]
mod observer_tests;
#[cfg(test)]
#[path = "ops_tests.rs"]
mod ops_tests;
#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod pipeline_tests;
#[cfg(test)]
mod test_support;
