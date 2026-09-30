//! The model the step loop talks to.
//!
//! [`StepModel`] is the seam: one prompt in, one reply out, with gate errors
//! kept distinct from everything else because the loop handles them
//! differently (a preempted call is requeued, a failed one is retried and then
//! fails the step). The production implementation wraps the `mlx:` chat model
//! built by the provider factory, which is already behind the single-flight
//! gate, so nothing here takes the gate itself; its permit is not re-entrant.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ChatModel, ModelRequest};

use crate::neppy::config::Config;

use super::super::gated_model::gate_error_of;
use super::super::service::mlx_admin::gate::GateError;

/// A whole step's model call, gate wait and first load included, must finish
/// inside this.
const CALL_TIMEOUT: Duration = Duration::from_secs(900);
/// Low, for output that has to parse.
const TEMPERATURE: f64 = 0.2;

#[derive(Debug, Clone)]
pub(crate) struct ModelReply {
    pub(crate) text: String,
    pub(crate) prompt_tokens: Option<u64>,
    pub(crate) completion_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) enum ModelFailure {
    /// The gate refused, paused or preempted the call.
    Gate(GateError),
    Other(String),
}

#[async_trait]
pub(crate) trait StepModel: Send + Sync {
    async fn complete(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ModelReply, ModelFailure>;
}

pub(crate) struct MlxStepModel {
    model: Arc<dyn ChatModel<()>>,
    label: String,
}

#[async_trait]
impl StepModel for MlxStepModel {
    async fn complete(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<ModelReply, ModelFailure> {
        let request = ModelRequest {
            messages: vec![Message::system(system), Message::user(user)],
            max_tokens: Some(max_tokens),
            temperature: Some(TEMPERATURE),
            // Hidden reasoning would spend the step's token budget before the
            // JSON starts.
            provider_options: serde_json::json!({
                "chat_template_kwargs": { "enable_thinking": false }
            }),
            ..ModelRequest::default()
        };
        log::debug!(
            "[local_assistant:model] {} call: prompt_chars={} max_tokens={max_tokens}",
            self.label,
            system.len() + user.len()
        );
        let response =
            match tokio::time::timeout(CALL_TIMEOUT, self.model.invoke(&(), request)).await {
                Ok(Ok(response)) => response,
                Ok(Err(err)) => {
                    return Err(match gate_error_of(&err) {
                        Some(gate) => ModelFailure::Gate(gate),
                        None => ModelFailure::Other(err.to_string()),
                    })
                }
                Err(_) => {
                    return Err(ModelFailure::Other(format!(
                        "the model call timed out after {}s",
                        CALL_TIMEOUT.as_secs()
                    )))
                }
            };
        let usage = response.usage.or(response.message.usage);
        let text = Message::Assistant(response.message).text();
        if text.trim().is_empty() {
            return Err(ModelFailure::Other(
                "the model returned an empty reply".into(),
            ));
        }
        Ok(ModelReply {
            text,
            prompt_tokens: usage.map(|u| u.input_tokens).filter(|n| *n > 0),
            completion_tokens: usage.map(|u| u.output_tokens).filter(|n| *n > 0),
        })
    }
}

/// The model id the assistant runs: `local_assistant.model`, else the model of
/// the primary `[[mlx.server]]` block.
pub(crate) fn primary_model_id(config: &Config) -> Result<String, String> {
    let configured = config.local_assistant.model.trim();
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    let server = config
        .mlx
        .server("primary")
        .or_else(|| config.mlx.servers.first())
        .ok_or("no [[mlx.server]] block is configured and local_assistant.model is empty")?;
    if server.model.trim().is_empty() {
        return Err("the primary MLX server has no model selected".into());
    }
    Ok(server.model.trim().to_string())
}

fn build_one(config: &Config, model_id: &str) -> Result<Arc<dyn StepModel>, String> {
    let (model, _) =
        crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
            &format!("mlx:{model_id}"),
            config,
        )
        .map_err(|err| format!("could not build the local model `{model_id}`: {err}"))?;
    Ok(Arc::new(MlxStepModel {
        model,
        label: model_id.to_string(),
    }))
}

type Models = (Arc<dyn StepModel>, Option<Arc<dyn StepModel>>);

/// The primary model and, when `mlx.worker.fallback_model` names a different
/// one, the fallback used if the primary cannot be admitted.
pub(crate) fn build_models(config: &Config) -> Result<Models, String> {
    let primary_id = primary_model_id(config)?;
    let primary = build_one(config, &primary_id)?;
    let fallback_id = config.mlx.worker.fallback_model.trim();
    let fallback = if fallback_id.is_empty() || fallback_id == primary_id {
        None
    } else {
        match build_one(config, fallback_id) {
            Ok(model) => Some(model),
            Err(err) => {
                log::warn!("[local_assistant:model] fallback unavailable: {err}");
                None
            }
        }
    };
    Ok((primary, fallback))
}
