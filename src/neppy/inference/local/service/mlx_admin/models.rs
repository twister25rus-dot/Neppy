//! The local model cache: what is on disk, and how to reclaim it.
//!
//! Deliberately not a downloader. `mlx_vlm.server` fetches a checkpoint it
//! does not have when a request names one, and `model_discovery = "hf-cache"`
//! already makes `/v1/models` enumerate what is cached. Reimplementing
//! Hugging Face downloads here would duplicate that, add a network egress
//! surface, and go stale against the Hub's own client.
//!
//! What the server cannot answer is what a checkpoint costs on disk and how to
//! get that space back. A 27B checkpoint is ~12 GB, and three quantizations of
//! the same model are easy to accumulate without noticing.
//!
//! Deletion moves the directory to the Trash rather than unlinking it. These
//! are multi-gigabyte downloads over a metered connection for some users, and
//! an unrecoverable delete driven by one mis-aimed click is a bad trade for
//! the milliseconds saved.

use std::path::{Path, PathBuf};

use serde::Serialize;

const BYTES_PER_GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// One cached Hugging Face repo.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct CachedModel {
    /// Repo id as the server names it, e.g. `mlx-community/Qwen3.8-27B-nvfp4`.
    pub(crate) id: String,
    /// On-disk size of the weights, GiB.
    pub(crate) size_gib: f64,
    /// Whether the id or its files suggest an MLX checkpoint. Advisory: the
    /// cache is shared with every other Hugging Face consumer on the machine,
    /// so this marks what is plausibly usable here rather than filtering the
    /// listing down to a guess.
    pub(crate) looks_like_mlx: bool,
}

/// The cache as a whole.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CacheListing {
    pub(crate) models: Vec<CachedModel>,
    pub(crate) total_gib: f64,
    /// Directory scanned, so the UI can say where the space went.
    pub(crate) cache_dir: String,
}

/// Hugging Face cache directory, honouring the standard env overrides.
pub(crate) fn hf_hub_dir() -> PathBuf {
    if let Some(hub) = std::env::var_os("HF_HUB_CACHE") {
        return PathBuf::from(hub);
    }
    if let Some(hf_home) = std::env::var_os("HF_HOME") {
        return PathBuf::from(hf_home).join("hub");
    }
    home_dir().join(".cache").join("huggingface").join("hub")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `models--org--name` → `org/name`.
///
/// Hugging Face flattens `/` to `--`, and an org name may itself contain `-`,
/// so only the first `--` after the prefix separates org from repo. A bare
/// name with no org has no separator at all.
pub(crate) fn repo_id_from_dir(dir_name: &str) -> Option<String> {
    let rest = dir_name.strip_prefix("models--")?;
    match rest.split_once("--") {
        Some((org, name)) => Some(format!("{org}/{name}")),
        None => Some(rest.to_string()),
    }
}

/// `org/name` → `models--org--name`.
pub(crate) fn dir_name_from_repo_id(repo_id: &str) -> String {
    format!("models--{}", repo_id.trim().replace('/', "--"))
}

/// Heuristic for "this is probably an MLX checkpoint".
///
/// Advisory only, and intentionally loose: the naming conventions are
/// community convention, not a standard.
fn looks_like_mlx(repo_id: &str) -> bool {
    let lowered = repo_id.to_ascii_lowercase();
    lowered.contains("mlx")
        || lowered.contains("nvfp4")
        || lowered.contains("-4bit")
        || lowered.contains("-8bit")
}

/// Total size of the regular files directly inside `dir`.
fn directory_size_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut total = 0u64;
    for entry in entries.flatten() {
        if let Ok(metadata) = entry.metadata() {
            if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

/// Every cached repo, largest first.
///
/// Sizes come from each repo's `blobs` directory: `snapshots` holds symlinks
/// into it, so measuring both would double count.
pub(crate) fn list_cached_models() -> CacheListing {
    let cache_dir = hf_hub_dir();
    let mut models = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let Some(id) = repo_id_from_dir(&dir_name) else {
                continue;
            };
            let bytes = directory_size_bytes(&entry.path().join("blobs"));
            if bytes == 0 {
                continue;
            }
            models.push(CachedModel {
                looks_like_mlx: looks_like_mlx(&id),
                id,
                size_gib: bytes as f64 / BYTES_PER_GIB,
            });
        }
    }

    // Largest first: the listing exists to answer "what can I delete", and the
    // answer is almost always at the top.
    models.sort_by(|a, b| {
        b.size_gib
            .partial_cmp(&a.size_gib)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let total_gib = models.iter().map(|model| model.size_gib).sum();
    CacheListing {
        models,
        total_gib,
        cache_dir: cache_dir.display().to_string(),
    }
}

/// Move a cached repo to the Trash.
///
/// Returns the reclaimed size in GiB. Refuses anything that is not a direct
/// `models--*` child of the cache directory: the id reaches this from an RPC
/// param, and a traversal here would delete an arbitrary directory.
pub(crate) fn delete_cached_model(repo_id: &str) -> Result<f64, String> {
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Err("no model id given".to_string());
    }

    let dir_name = dir_name_from_repo_id(repo_id);
    // `models--` is already a prefix, so the only way to escape is a separator
    // inside the id itself. Reject rather than sanitise: a caller sending one
    // is not asking for something we should guess at.
    if dir_name.contains('/') || dir_name.contains("..") || dir_name.contains('\\') {
        return Err(format!("`{repo_id}` is not a valid model id"));
    }

    let cache_dir = hf_hub_dir();
    let target = cache_dir.join(&dir_name);
    if !target.is_dir() {
        return Err(format!("`{repo_id}` is not in the local cache"));
    }

    // Confirm the resolved path really sits under the cache root, so a symlink
    // in the cache directory cannot redirect the delete elsewhere.
    let canonical_target = target
        .canonicalize()
        .map_err(|err| format!("could not resolve `{repo_id}`: {err}"))?;
    let canonical_cache = cache_dir
        .canonicalize()
        .map_err(|err| format!("could not resolve the cache directory: {err}"))?;
    if !canonical_target.starts_with(&canonical_cache) {
        return Err(format!(
            "`{repo_id}` resolves outside the model cache; refusing to delete it"
        ));
    }

    let reclaimed = directory_size_bytes(&canonical_target.join("blobs")) as f64 / BYTES_PER_GIB;

    trash_directory(&canonical_target)?;
    log::info!("[mlx] moved `{repo_id}` to the trash, reclaiming {reclaimed:.1} GiB");
    Ok(reclaimed)
}

/// Move `path` to the platform's trash, falling back to a rename into the
/// user's Trash directory on macOS.
#[cfg(target_os = "macos")]
fn trash_directory(path: &Path) -> Result<(), String> {
    let trash = home_dir().join(".Trash");
    if !trash.is_dir() {
        return Err("no Trash directory to move the model into".to_string());
    }

    let stem = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "model".to_string());

    // A repo deleted twice would collide in the Trash, so disambiguate.
    let mut destination = trash.join(&stem);
    let mut suffix = 1;
    while destination.exists() {
        destination = trash.join(format!("{stem}-{suffix}"));
        suffix += 1;
    }

    std::fs::rename(path, &destination)
        .map_err(|err| format!("could not move the model to the Trash: {err}"))
}

#[cfg(not(target_os = "macos"))]
fn trash_directory(path: &Path) -> Result<(), String> {
    // No portable trash on the other desktop targets Neppy ships to, and an
    // unrecoverable recursive delete is not something to do silently.
    Err(format!(
        "moving `{}` to the trash is only supported on macOS; delete it manually",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ids_round_trip_through_the_cache_layout() {
        assert_eq!(
            repo_id_from_dir("models--mlx-community--Qwen3.8-27B-nvfp4").as_deref(),
            Some("mlx-community/Qwen3.8-27B-nvfp4")
        );
        assert_eq!(
            dir_name_from_repo_id("mlx-community/Qwen3.8-27B-nvfp4"),
            "models--mlx-community--Qwen3.8-27B-nvfp4"
        );
    }

    #[test]
    fn only_the_first_separator_splits_org_from_name() {
        // The repo name itself contains hyphens, and an org may too. Splitting
        // on the last `--` would produce a different, wrong id.
        assert_eq!(
            repo_id_from_dir("models--LiquidAI--LFM2.5-1.2B-Instruct-MLX-4bit").as_deref(),
            Some("LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit")
        );
    }

    #[test]
    fn a_repo_without_an_org_still_resolves() {
        assert_eq!(repo_id_from_dir("models--gpt2").as_deref(), Some("gpt2"));
        assert_eq!(dir_name_from_repo_id("gpt2"), "models--gpt2");
    }

    #[test]
    fn non_model_directories_are_ignored() {
        assert_eq!(repo_id_from_dir("datasets--squad"), None);
        assert_eq!(repo_id_from_dir(".locks"), None);
    }

    #[test]
    fn the_mlx_heuristic_recognises_the_usual_conventions() {
        assert!(looks_like_mlx("mlx-community/Qwen3.8-27B-nvfp4"));
        assert!(looks_like_mlx("LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit"));
        assert!(looks_like_mlx("YoozLabs/Qwen3.8-27B-lean-4bit-mlx"));
        assert!(!looks_like_mlx("openai/whisper-large-v3"));
    }

    #[test]
    fn deleting_refuses_a_traversal_attempt() {
        // The id arrives as an RPC parameter.
        for hostile in ["../../etc", "..", "a/../../b", "x\\y"] {
            let result = delete_cached_model(hostile);
            assert!(
                result.is_err(),
                "`{hostile}` should be refused, got {result:?}"
            );
        }
    }

    #[test]
    fn deleting_an_absent_model_says_so() {
        let result = delete_cached_model("definitely/not-cached-xyz");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in the local cache"));
    }

    #[test]
    fn deleting_nothing_is_refused() {
        assert!(delete_cached_model("   ").is_err());
    }

    #[test]
    fn the_listing_is_ordered_largest_first() {
        // Runs against the real cache: on a machine with none, the listing is
        // empty and the ordering claim is vacuously true.
        let listing = list_cached_models();
        for pair in listing.models.windows(2) {
            assert!(
                pair[0].size_gib >= pair[1].size_gib,
                "listing must be ordered largest first"
            );
        }
        let summed: f64 = listing.models.iter().map(|model| model.size_gib).sum();
        assert!((summed - listing.total_gib).abs() < 0.001);
    }
}
