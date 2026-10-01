use std::path::PathBuf;
use std::sync::Arc;

use crate::neppy::config::Config;

use super::model_ids::effective_chat_model_id;
use super::service::LocalAiService;

static LOCAL_AI: once_cell::sync::OnceCell<Arc<LocalAiService>> = once_cell::sync::OnceCell::new();

pub fn global(config: &Config) -> Arc<LocalAiService> {
    LOCAL_AI
        .get_or_init(|| Arc::new(LocalAiService::new(config)))
        .clone()
}

/// Like [`global`] but returns `None` instead of initialising the singleton.
///
/// Useful from shutdown paths where lazy-creating the service just to call a
/// no-op cleanup would be wasteful — if local AI was never used in this
/// process, there's nothing to clean up.
pub fn try_global() -> Option<Arc<LocalAiService>> {
    LOCAL_AI.get().cloned()
}

/// Stop every MLX worker this process spawned, gracefully and within a bound.
///
/// Works whether or not the local AI service exists: the process-wide worker
/// registry is swept first (needs neither the pool nor a config), then the
/// pool's own map is drained. Idempotent, so the signal hook, the post-serve
/// cleanup and the desktop teardown can all call it. Returns how many worker
/// processes were stopped.
pub async fn shutdown_mlx_workers() -> usize {
    use super::service::mlx_admin::reaper;
    let stop = async {
        let swept = reaper::shutdown_tracked_workers(reaper::SHUTDOWN_GRACE).await;
        let pooled = match try_global() {
            Some(svc) => svc.mlx.shutdown_all(reaper::SHUTDOWN_GRACE).await,
            None => 0,
        };
        swept + pooled
    };
    match tokio::time::timeout(reaper::SHUTDOWN_BUDGET, stop).await {
        Ok(n) => {
            log::info!("[mlx] shutdown: stopped {n} worker process(es)");
            n
        }
        Err(_) => {
            log::warn!("[mlx] shutdown: worker stop exceeded its budget; proceeding with exit");
            0
        }
    }
}

/// Synchronous form for a caller with no async context, such as the desktop's
/// `RunEvent::ExitRequested` teardown. Blocks for at most about two seconds.
pub fn shutdown_mlx_workers_blocking() -> usize {
    use super::service::mlx_admin::reaper;
    reaper::shutdown_tracked_workers_blocking(reaper::SHUTDOWN_GRACE)
}

/// Make this process responsible for the MLX workers it spawns.
///
/// Called once by a long-lived host (`serve`, the embedded desktop core), never
/// by a one-shot CLI call, whose servers are meant to outlive it. It
/// - marks workers spawned from now on as `supervised`, so a later boot may
///   reap them if this process dies without cleaning up;
/// - registers a shutdown hook, so SIGTERM/SIGINT stop the workers at once
///   rather than after in-flight connections drain (which can be never);
/// - reaps workers a previous supervised host left behind.
pub fn supervise_mlx_workers() {
    use super::service::mlx_admin::reaper;
    static ONCE: std::sync::Once = std::sync::Once::new();
    reaper::declare_supervised_host();
    ONCE.call_once(|| {
        crate::core::shutdown::register(|| async {
            shutdown_mlx_workers().await;
        });
    });
    // Reading markers and probing PIDs is blocking work; keep it off the
    // server's start path and off the async workers.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async {
            let config = match Config::load_or_init().await {
                Ok(config) => config,
                Err(err) => {
                    log::debug!("[mlx:reaper] boot reap skipped: cannot load config: {err}");
                    return;
                }
            };
            match tokio::task::spawn_blocking(move || reaper::reap_stale_workers(&config)).await {
                Ok(0) => log::debug!("[mlx:reaper] boot reap: nothing stale"),
                Ok(n) => log::warn!("[mlx:reaper] boot reap: stopped {n} orphaned worker(s)"),
                Err(err) => log::warn!("[mlx:reaper] boot reap task failed: {err}"),
            }
        });
    }
}

pub fn model_artifact_path(config: &Config) -> PathBuf {
    let root = crate::neppy::config::default_root_neppy_dir().unwrap_or_else(|_| {
        config
            .config_path
            .parent()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| config.workspace_dir.clone())
    });
    root.join("models")
        .join("local-ai")
        .join(effective_chat_model_id(config).replace(':', "-") + ".ollama")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_artifact_path_includes_models_local_ai_subdirs() {
        let config = Config::default();
        let path = model_artifact_path(&config);
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("models"),
            "expected `models` in path: {path_str}"
        );
        assert!(
            path_str.contains("local-ai"),
            "expected `local-ai` subdir in path: {path_str}"
        );
    }

    #[test]
    fn model_artifact_path_ends_with_ollama_suffix() {
        let config = Config::default();
        let path = model_artifact_path(&config);
        assert_eq!(
            path.extension().and_then(|s| s.to_str()),
            Some("ollama"),
            "model artifact must have `.ollama` extension: {}",
            path.display()
        );
    }

    #[test]
    fn model_artifact_path_replaces_colon_in_model_id_with_dash() {
        // Model IDs commonly look like `qwen2:1.5b`; colons are illegal on
        // Windows path components, so we normalise to `-`. This test pins
        // that mapping.
        let config = Config::default();
        let path = model_artifact_path(&config);
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        assert!(!file.contains(':'), "filename must not contain `:`: {file}");
    }

    #[test]
    fn global_returns_same_arc_across_calls() {
        let config = Config::default();
        let a = global(&config);
        let b = global(&config);
        assert!(Arc::ptr_eq(&a, &b), "global() must return a shared Arc");
    }
}
