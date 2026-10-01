//! `[mlx.worker]` — how the managed MLX worker is kept, admitted and stopped.
//!
//! Consumed by `inference::local::service::mlx_admin::{gate, pressure,
//! watchdog, metrics}`. Every field has a default, so an existing
//! `config.toml` without this table deserializes unchanged and needs no
//! migration.
//!
//! The defaults encode the policy the local-assistant plan measured for a
//! 36 GiB machine: one active request, the model unloaded after five idle
//! minutes and the process stopped after fifteen, and a memory threshold with
//! hysteresis so a machine hovering at the edge does not flap.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `idle_policy` value: unload/stop on idle timers and on pressure.
pub const IDLE_POLICY_ALWAYS: &str = "always";
/// `idle_policy` value: keep the model resident until memory pressure rises.
pub const IDLE_POLICY_PRESSURE_ONLY: &str = "pressure_only";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MlxWorkerConfig {
    /// `always` (default) or `pressure_only`. Anything else reads as `always`.
    pub idle_policy: String,
    /// Seconds without a request before the loaded model is unloaded
    /// (`POST /unload`, process kept). `0` disables the unload timer.
    pub idle_unload_secs: u64,
    /// Seconds without a request before the worker process is stopped.
    /// `0` disables the stop timer.
    pub idle_stop_secs: u64,
    /// Available memory, percent, below which the state becomes Elevated.
    pub elevated_avail_pct: f64,
    /// Available memory, percent, below which the state becomes Critical.
    pub critical_avail_pct: f64,
    /// Available memory, percent, that must hold before returning to Normal.
    /// `0.0` (the default) means `elevated_avail_pct + 5`, so a machine that
    /// normally sits just above the Elevated line is not paused indefinitely.
    /// An explicit value is never taken below `elevated_avail_pct`.
    pub recover_avail_pct: f64,
    /// Seconds the recovery condition must hold before returning to Normal.
    pub recover_hold_secs: u64,
    /// Memory, GiB, that must stay available after a managed (lazy) model load.
    /// `0.0` means `clamp(15% of physical memory, 2 GiB, 8 GiB)`. An explicit
    /// `mlx.start` is held only to the memory budget and a 2 GiB floor.
    pub reserve_gib: f64,
    /// Callers allowed to wait for the single inference slot. One more is
    /// refused as busy rather than queued without bound.
    pub max_waiters: usize,
    /// Seconds a caller waits for the inference slot before giving up.
    pub acquire_timeout_secs: u64,
    /// Seconds a lazily started worker has to become ready.
    pub first_token_timeout_secs: u64,
    /// Crash restarts allowed in a sliding ten-minute window.
    pub max_restarts_per_10min: u32,
    /// Smaller model to fall back to when the configured one is refused
    /// admission. Empty means no fallback. Consumed by the assistant (T2).
    pub fallback_model: String,
    /// Daily metrics files kept on disk.
    pub metrics_retention_days: u32,
    /// Size cap, MiB, of one day's metrics file.
    pub metrics_file_cap_mib: u64,
}

impl Default for MlxWorkerConfig {
    fn default() -> Self {
        Self {
            idle_policy: IDLE_POLICY_ALWAYS.to_string(),
            idle_unload_secs: 300,
            idle_stop_secs: 900,
            elevated_avail_pct: 20.0,
            critical_avail_pct: 10.0,
            recover_avail_pct: 0.0,
            recover_hold_secs: 30,
            reserve_gib: 0.0,
            max_waiters: 4,
            acquire_timeout_secs: 300,
            first_token_timeout_secs: 180,
            max_restarts_per_10min: 3,
            fallback_model: String::new(),
            metrics_retention_days: 7,
            metrics_file_cap_mib: 20,
        }
    }
}

impl MlxWorkerConfig {
    /// Whether the model stays resident until pressure rises.
    pub fn pressure_only(&self) -> bool {
        self.idle_policy
            .trim()
            .eq_ignore_ascii_case(IDLE_POLICY_PRESSURE_ONLY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_table_takes_every_default() {
        let parsed: MlxWorkerConfig = toml::from_str("").expect("empty table parses");
        assert_eq!(parsed, MlxWorkerConfig::default());
        assert_eq!(parsed.idle_unload_secs, 300);
        assert_eq!(parsed.idle_stop_secs, 900);
        assert_eq!(parsed.max_waiters, 4);
        assert!(!parsed.pressure_only());
    }

    #[test]
    fn a_partial_table_keeps_the_other_defaults() {
        let parsed: MlxWorkerConfig =
            toml::from_str("idle_policy = \"Pressure_Only\"\nidle_unload_secs = 45\n")
                .expect("partial table parses");
        assert!(parsed.pressure_only());
        assert_eq!(parsed.idle_unload_secs, 45);
        assert_eq!(parsed.idle_stop_secs, 900);
        assert_eq!(parsed.recover_hold_secs, 30);
    }

    #[test]
    fn an_unknown_policy_reads_as_always() {
        let config = MlxWorkerConfig {
            idle_policy: "sometimes".to_string(),
            ..MlxWorkerConfig::default()
        };
        assert!(!config.pressure_only());
    }
}
