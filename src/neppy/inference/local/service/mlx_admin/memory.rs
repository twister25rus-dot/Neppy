//! Memory admission control.
//!
//! An MLX server holds its weights in unified memory, so two 27B checkpoints
//! do not fit on a 36 GB machine and the second one does not fail cleanly — it
//! swaps, and the whole desktop becomes unusable. Admission is therefore a
//! precondition of spawning, not a metric collected afterwards.
//!
//! The estimate comes from the Hugging Face cache. A model's `blobs` directory
//! is the quantized weights on disk, which is the dominant term in resident
//! size; KV cache and activations sit on top, so the estimate is scaled up
//! rather than used raw. It is an approximation, and deliberately a
//! conservative one: refusing a start that would have just fitted is a message
//! the user can act on, while admitting one that does not fit wedges the
//! machine.

use std::path::PathBuf;

use crate::neppy::config::Config;

// The Hugging Face cache layout has one owner: `models`. Encode and decode of
// `models--org--name` have to agree, and a second copy of that rule here is a
// drift waiting to happen.
use super::models::{dir_name_from_repo_id, directory_size_bytes, hf_hub_dir, BYTES_PER_GIB};
use super::pressure::{reserve_bytes, sample_process, sample_system, SystemMemory};

/// Fraction of physical memory available to MLX when no budget is configured.
/// The rest is macOS, Neppy itself, and whatever else is open.
const DEFAULT_BUDGET_FRACTION: f64 = 0.70;

/// Multiplier over on-disk weight size to account for KV cache, activations
/// and allocator overhead.
const RESIDENT_OVERHEAD: f64 = 1.20;

/// Physical memory on this machine, in GiB.
pub(crate) fn physical_memory_gib() -> f64 {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    system.total_memory() as f64 / BYTES_PER_GIB
}

/// The ceiling all MLX processes share.
pub(crate) fn budget_gib(config: &Config) -> f64 {
    let configured = config.mlx.memory_budget_gib;
    if configured > 0.0 {
        return configured;
    }
    physical_memory_gib() * DEFAULT_BUDGET_FRACTION
}

/// Resident memory of the given PIDs, in GiB.
///
/// Uses `phys_footprint` on macOS, which counts the Metal buffers an MLX
/// server's weights live in; RSS undercounts them. Elsewhere this is RSS.
pub(crate) fn resident_gib(pids: &[u32]) -> f64 {
    pids.iter()
        .filter_map(|pid| sample_process(*pid))
        .map(|mem| mem.footprint_bytes as f64 / BYTES_PER_GIB)
        .sum()
}

/// Estimated resident size of `model_id`, in GiB, or `None` when it is not in
/// the cache.
///
/// A local filesystem path is measured directly; anything else is looked up as
/// a Hugging Face repo id.
pub(crate) fn estimate_model_gib(model_id: &str) -> Option<f64> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return None;
    }

    let direct = PathBuf::from(model_id);
    let weights_dir = if direct.is_dir() {
        direct
    } else {
        // Weights live in `blobs`; `snapshots` holds symlinks into it, so
        // measuring blobs avoids double counting or following links.
        hf_hub_dir()
            .join(dir_name_from_repo_id(model_id))
            .join("blobs")
    };

    // Zero covers both "no such directory" and "empty", which are the same
    // answer here: nothing cached to measure.
    let bytes = directory_size_bytes(&weights_dir);
    if bytes == 0 {
        return None;
    }
    Some((bytes as f64 / BYTES_PER_GIB) * RESIDENT_OVERHEAD)
}

/// Verdict on whether a server may start.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Admission {
    /// Enough headroom; the estimate is carried for the UI.
    Allow { estimated_gib: f64 },
    /// Not enough headroom. `message` names the shortfall in the user's terms.
    Refuse { message: String },
    /// The model is not cached, so its size is unknown. Starting is allowed —
    /// the server will download it, and refusing on an unknown would block
    /// every first run.
    Unknown,
}

/// Decide whether a server holding `model_id` may start, given what is already
/// resident in `running_pids` and what the system has available right now.
pub(crate) fn admit(config: &Config, model_id: &str, running_pids: &[u32]) -> Admission {
    admit_with(config, model_id, running_pids, &sample_system())
}

/// [`admit`] with the system reading injected, so both checks are testable.
///
/// Two ceilings, both must hold:
/// - the MLX budget (`memory_budget_gib`, or 70% of RAM) minus what managed
///   servers already hold;
/// - what the machine actually has available minus a reserve
///   (`[mlx.worker] reserve_gib`, or `max(8 GiB, 25% of RAM)`). The budget
///   alone ignores every other app, so it would admit a load that swaps.
pub(crate) fn admit_with(
    config: &Config,
    model_id: &str,
    running_pids: &[u32],
    system: &SystemMemory,
) -> Admission {
    let Some(estimated_gib) = estimate_model_gib(model_id) else {
        log::debug!("[mlx] admission: `{model_id}` is not cached, size unknown — allowing");
        return Admission::Unknown;
    };

    let budget = budget_gib(config);
    let in_use = resident_gib(running_pids);
    let free = budget - in_use;

    log::debug!(
        "[mlx] admission: model={model_id} needs={estimated_gib:.1}GiB \
         in_use={in_use:.1}GiB budget={budget:.1}GiB free={free:.1}GiB \
         avail={:.1}GiB",
        system.avail_bytes as f64 / BYTES_PER_GIB
    );

    if estimated_gib > free {
        // Say what would have to happen, not merely that it failed.
        let advice = if in_use > 0.0 {
            " Stop another MLX server first, or raise mlx.memory_budget_gib."
        } else {
            " Raise mlx.memory_budget_gib, or choose a smaller quantization."
        };

        return Admission::Refuse {
            message: format!(
                "{model_id} needs about {estimated_gib:.1} GiB but only {free:.1} GiB of the \
                 {budget:.1} GiB MLX budget is free.{advice}"
            ),
        };
    }

    if system.total_bytes > 0 {
        let avail = system.avail_bytes as f64 / BYTES_PER_GIB;
        let reserve = reserve_bytes(&config.mlx.worker, system.total_bytes) as f64 / BYTES_PER_GIB;
        if avail - estimated_gib < reserve {
            return Admission::Refuse {
                message: format!(
                    "{model_id} needs about {estimated_gib:.1} GiB but only {avail:.1} GiB of \
                     memory is available and {reserve:.1} GiB must stay free for the system. \
                     Close other apps, lower mlx.worker.reserve_gib, or choose a smaller \
                     quantization."
                ),
            };
        }
    }

    Admission::Allow { estimated_gib }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uncached_model_is_unknown_not_refused() {
        // Refusing an unknown size would block every first run, before the
        // server has had a chance to download anything.
        let config = Config::default();
        assert_eq!(
            admit(&config, "definitely/not-a-real-model-xyz", &[]),
            Admission::Unknown
        );
    }

    #[test]
    fn an_empty_model_slot_has_no_estimate() {
        assert_eq!(estimate_model_gib(""), None);
        assert_eq!(estimate_model_gib("   "), None);
    }

    #[test]
    fn the_budget_defaults_to_a_fraction_of_physical_memory() {
        let mut config = Config::default();
        config.mlx.memory_budget_gib = 0.0;

        let derived = budget_gib(&config);
        let physical = physical_memory_gib();
        assert!(derived < physical, "budget must leave headroom for the OS");
        assert!(derived > 0.0);

        config.mlx.memory_budget_gib = 12.5;
        assert_eq!(budget_gib(&config), 12.5);
    }

    fn fake_model_dir(bytes: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("weights.safetensors"), vec![0u8; bytes]).expect("write");
        dir
    }

    fn system(total_gib: f64, avail_gib: f64) -> SystemMemory {
        let gib = BYTES_PER_GIB;
        SystemMemory {
            total_bytes: (total_gib * gib) as u64,
            avail_pct: avail_gib * 100.0 / total_gib,
            avail_bytes: (avail_gib * gib) as u64,
            pressure_level: 1,
            ..SystemMemory::default()
        }
    }

    #[test]
    fn admission_refuses_when_the_system_reserve_would_be_breached() {
        let dir = fake_model_dir(1024 * 1024);
        let model = dir.path().to_string_lossy().into_owned();
        let mut config = Config::default();
        config.mlx.memory_budget_gib = 100.0;
        config.mlx.worker.reserve_gib = 9.0;

        match admit_with(&config, &model, &[], &system(36.0, 9.0005)) {
            Admission::Refuse { message } => {
                assert!(message.contains("must stay free"), "{message}");
                assert!(message.contains("reserve_gib"), "{message}");
            }
            other => panic!("expected refusal, got {other:?}"),
        }
        assert!(matches!(
            admit_with(&config, &model, &[], &system(36.0, 20.0)),
            Admission::Allow { .. }
        ));
    }

    #[test]
    fn admission_uses_the_derived_reserve_by_default() {
        let dir = fake_model_dir(1024);
        let model = dir.path().to_string_lossy().into_owned();
        let mut config = Config::default();
        config.mlx.memory_budget_gib = 100.0;
        // 36 GiB total: reserve is max(8, 9) = 9 GiB.
        assert!(matches!(
            admit_with(&config, &model, &[], &system(36.0, 8.5)),
            Admission::Refuse { .. }
        ));
        assert!(matches!(
            admit_with(&config, &model, &[], &system(36.0, 9.5)),
            Admission::Allow { .. }
        ));
        // An unreadable system reading does not block on the reserve.
        assert!(matches!(
            admit_with(&config, &model, &[], &SystemMemory::default()),
            Admission::Allow { .. }
        ));
    }

    #[test]
    fn the_budget_check_still_comes_first() {
        let dir = fake_model_dir(1024 * 1024);
        let model = dir.path().to_string_lossy().into_owned();
        let mut config = Config::default();
        config.mlx.memory_budget_gib = 0.000_001;
        match admit_with(&config, &model, &[], &system(36.0, 30.0)) {
            Admission::Refuse { message } => assert!(message.contains("memory_budget_gib")),
            other => panic!("expected refusal, got {other:?}"),
        }
    }

    #[test]
    fn resident_usage_of_our_own_process_is_positive() {
        assert!(resident_gib(&[std::process::id()]) > 0.0);
    }

    #[test]
    fn resident_usage_of_no_processes_is_zero() {
        assert_eq!(resident_gib(&[]), 0.0);
    }

    #[test]
    fn a_model_larger_than_the_budget_is_refused_with_a_next_step() {
        // Measure a real cached model when one is present; otherwise the
        // estimator has nothing to work from and the case is Unknown, which
        // the test above already covers.
        let mut config = Config::default();
        config.mlx.memory_budget_gib = 0.001;

        let cached = "mlx-community/Qwen3.8-27B-nvfp4";
        if estimate_model_gib(cached).is_some() {
            match admit(&config, cached, &[]) {
                Admission::Refuse { message } => {
                    assert!(
                        message.contains("GiB"),
                        "message should quantify: {message}"
                    );
                    assert!(
                        message.contains("memory_budget_gib"),
                        "message should name the next step: {message}"
                    );
                }
                other => panic!("expected a refusal, got {other:?}"),
            }
        }
    }
}
