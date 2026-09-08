//! `LocalAiService` methods for the MLX runtime.
//!
//! Kept beside the free functions rather than inside them because bootstrap
//! reaches the runtime through `LocalAiService`, exactly as it does for Ollama
//! (`ollama_admin`) and LM Studio (`service::lm_studio`).

use crate::neppy::config::Config;
use crate::neppy::inference::local::mlx::{mlx_api_key, mlx_base_url};

use super::super::LocalAiService;
use super::health::probe_models;

impl LocalAiService {
    /// Confirm an MLX server is reachable and speaking the OpenAI surface.
    ///
    /// Connectivity only — whether a model is loaded is a separate question,
    /// surfaced through asset status so the UI can offer "load a model" rather
    /// than failing the whole bootstrap. Same shape as
    /// `ensure_lm_studio_available`.
    pub(in crate::neppy::inference::local::service) async fn ensure_mlx_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        let base_url = mlx_base_url(config);
        let api_key = mlx_api_key(config);

        log::debug!("[local_ai:mlx] availability check start endpoint={base_url}");

        let report = probe_models(&self.http, &base_url, api_key.as_deref()).await;
        if report.reachable {
            log::debug!(
                "[local_ai:mlx] availability check succeeded endpoint={base_url} models={}",
                report.models.len()
            );
            return Ok(());
        }

        let detail = report.detail.unwrap_or_else(|| "no response".to_string());
        log::debug!("[local_ai:mlx] availability check failed endpoint={base_url}: {detail}");

        // Name the endpoint. The commonest cause is a supervised block with
        // `port = 0`, whose real port only the pool knows, so the probe went to
        // the profile default and found nothing.
        Err(format!(
            "MLX server at {base_url} is not reachable: {detail}. \
             Start it from Settings, or set a fixed port on the [[mlx.server]] block."
        ))
    }
}
