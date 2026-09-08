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

/// Fraction of physical memory available to MLX when no budget is configured.
/// The rest is macOS, Neppy itself, and whatever else is open.
const DEFAULT_BUDGET_FRACTION: f64 = 0.70;

/// Multiplier over on-disk weight size to account for KV cache, activations
/// and allocator overhead.
const RESIDENT_OVERHEAD: f64 = 1.20;

const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

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
pub(crate) fn resident_gib(pids: &[u32]) -> f64 {
    if pids.is_empty() {
        return 0.0;
    }
    let targets: Vec<sysinfo::Pid> = pids.iter().copied().map(sysinfo::Pid::from_u32).collect();
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&targets), true);
    targets
        .iter()
        .filter_map(|pid| system.process(*pid))
        .map(|process| process.memory() as f64 / BYTES_PER_GIB)
        .sum()
}

/// Hugging Face cache directory, honouring the standard env overrides.
fn hf_hub_dir() -> PathBuf {
    if let Some(hub) = std::env::var_os("HF_HUB_CACHE") {
        return PathBuf::from(hub);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    if let Some(hf_home) = std::env::var_os("HF_HOME") {
        return PathBuf::from(hf_home).join("hub");
    }
    home.join(".cache").join("huggingface").join("hub")
}

/// Cache directory name for a repo id: `org/name` → `models--org--name`.
fn cache_dir_name(model_id: &str) -> String {
    format!("models--{}", model_id.trim().replace('/', "--"))
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
        hf_hub_dir().join(cache_dir_name(model_id)).join("blobs")
    };

    let bytes = directory_size_bytes(&weights_dir)?;
    if bytes == 0 {
        return None;
    }
    Some((bytes as f64 / BYTES_PER_GIB) * RESIDENT_OVERHEAD)
}

/// Sum of regular-file sizes directly inside `dir`, recursing one level.
///
/// Deliberately shallow and symlink-blind: `blobs` is flat, and following
/// links would double count against `snapshots`.
fn directory_size_bytes(dir: &PathBuf) -> Option<u64> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Some(total)
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
/// resident in `running_pids`.
pub(crate) fn admit(config: &Config, model_id: &str, running_pids: &[u32]) -> Admission {
    let Some(estimated_gib) = estimate_model_gib(model_id) else {
        log::debug!("[mlx] admission: `{model_id}` is not cached, size unknown — allowing");
        return Admission::Unknown;
    };

    let budget = budget_gib(config);
    let in_use = resident_gib(running_pids);
    let free = budget - in_use;

    log::debug!(
        "[mlx] admission: model={model_id} needs={estimated_gib:.1}GiB \
         in_use={in_use:.1}GiB budget={budget:.1}GiB free={free:.1}GiB"
    );

    if estimated_gib <= free {
        return Admission::Allow { estimated_gib };
    }

    // Say what would have to happen, not merely that it failed.
    let advice = if in_use > 0.0 {
        " Stop another MLX server first, or raise mlx.memory_budget_gib."
    } else {
        " Raise mlx.memory_budget_gib, or choose a smaller quantization."
    };

    Admission::Refuse {
        message: format!(
            "{model_id} needs about {estimated_gib:.1} GiB but only {free:.1} GiB of the \
             {budget:.1} GiB MLX budget is free.{advice}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_name_matches_the_hugging_face_layout() {
        assert_eq!(
            cache_dir_name("mlx-community/Qwen3.8-27B-nvfp4"),
            "models--mlx-community--Qwen3.8-27B-nvfp4"
        );
        // A bare name has no org segment and still resolves.
        assert_eq!(cache_dir_name("gpt2"), "models--gpt2");
    }

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
