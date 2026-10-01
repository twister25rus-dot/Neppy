//! `GatedLocalModel` — every `mlx:` chat model, behind the single-flight gate.
//!
//! The provider factory wraps each `mlx:` model in this when the MLX
//! supervisor is on, so chat, the memory summariser, heartbeat, triage and the
//! assistant all share one inference slot. A call:
//!
//! 1. takes the gate permit (bounded wait, pausable under memory pressure);
//! 2. makes the worker ready — lazy start, crash restart, and unloading a
//!    different model so at most one is resident;
//! 3. runs the inner model, racing it against preemption.
//!
//! The permit is RAII and released on every path. A stream carries it and
//! releases it at its terminal item, or when the caller drops the stream
//! part-way — whichever comes first.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt;
use tinyagents::harness::model::{
    ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyagents::harness::usage::Usage;
use tinyagents::TinyAgentsError;

use crate::neppy::config::schema::MlxWorkerConfig;
use crate::neppy::config::Config;

use super::service::mlx_admin::gate::{GateError, GatePermit, InferenceGate};
use super::service::mlx_admin::metrics::{event, MetricsSink};
use super::service::LocalAiService;

/// Makes the worker ready for a model. Split out so tests can script it
/// without a real MLX server.
#[async_trait]
pub(crate) trait WorkerReady: Send + Sync {
    /// Prepare the worker for `model_id`. `Ok(true)` means this request will
    /// load the model, so its completion marks it ready.
    async fn prepare(&self, model_id: &str) -> Result<bool, String>;
}

/// The production [`WorkerReady`]: the supervisor on `LocalAiService`.
struct ServiceWorker {
    svc: Arc<LocalAiService>,
    config: Arc<Config>,
    /// `false` when the endpoint is an explicit override the supervisor does
    /// not own; the gate still applies.
    manage: bool,
}

#[async_trait]
impl WorkerReady for ServiceWorker {
    async fn prepare(&self, model_id: &str) -> Result<bool, String> {
        if !self.manage {
            return Ok(false);
        }
        use super::service::mlx_admin::worker::{ensure_started, prepare_model};
        ensure_started(&self.svc, &self.config).await?;
        prepare_model(&self.svc, &self.config, model_id).await
    }
}

/// A chat model that takes the inference gate around every call.
pub(crate) struct GatedLocalModel {
    inner: Arc<dyn ChatModel<()>>,
    model_id: String,
    gate: Arc<InferenceGate>,
    metrics: Arc<MetricsSink>,
    worker: Arc<dyn WorkerReady>,
    cfg: MlxWorkerConfig,
}

impl GatedLocalModel {
    pub(crate) fn new(
        inner: Arc<dyn ChatModel<()>>,
        model_id: impl Into<String>,
        gate: Arc<InferenceGate>,
        metrics: Arc<MetricsSink>,
        worker: Arc<dyn WorkerReady>,
        cfg: MlxWorkerConfig,
    ) -> Self {
        Self {
            inner,
            model_id: model_id.into(),
            gate,
            metrics,
            worker,
            cfg,
        }
    }

    /// Acquire the permit and prepare the worker, both preemptible.
    async fn admit(&self) -> tinyagents::Result<(GatePermit, bool)> {
        let permit = self.gate.acquire(&self.cfg).await.map_err(gate_error)?;
        log::debug!("[mlx:gate] model={} admitted", self.model_id);
        let loading = tokio::select! {
            biased;
            _ = permit.cancelled() => {
                self.metrics.event(event::REQUEST_PREEMPTED, None, "while preparing the worker");
                return Err(gate_error(permit.cancel_error()));
            }
            prepared = self.worker.prepare(&self.model_id) => prepared.map_err(|err| {
                log::warn!("[mlx:worker] model={} not ready: {err}", self.model_id);
                TinyAgentsError::Model(format!("[mlx:worker] {err}"))
            })?,
        };
        Ok((permit, loading))
    }
}

/// Wrap a gate error as a model error callers can classify with
/// [`gate_error_of`].
pub(crate) fn gate_error(err: GateError) -> TinyAgentsError {
    TinyAgentsError::Model(err.message())
}

/// Appended to a gate error raised after the inner model was already running,
/// so the caller can tell a cancellation that may have cost tokens from one
/// that happened while still queued or loading.
const GENERATING_MARK: &str = " [generating]";

/// [`gate_error`] for a cancellation that interrupted a running call.
fn gate_error_while_generating(err: GateError) -> TinyAgentsError {
    TinyAgentsError::Model(format!("{}{GENERATING_MARK}", err.message()))
}

/// Whether a gate error came from interrupting a call that had started
/// generating.
pub(crate) fn generation_started(err: &TinyAgentsError) -> bool {
    err.to_string().contains(GENERATING_MARK)
}

/// Recover a [`GateError`] from a model error, if the gate produced it.
pub(crate) fn gate_error_of(err: &TinyAgentsError) -> Option<GateError> {
    GateError::from_message(&err.to_string())
}

fn record_usage(metrics: &MetricsSink, usage: Option<Usage>) {
    if let Some(usage) = usage {
        metrics.set_last_usage(usage.input_tokens, usage.output_tokens);
    }
}

#[async_trait]
impl ChatModel<()> for GatedLocalModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    fn cache_identity(&self) -> Option<String> {
        // Gating changes when a call runs, never what it returns.
        self.inner.cache_identity()
    }

    async fn invoke(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelResponse> {
        let (permit, loading) = self.admit().await?;
        let started = Instant::now();
        let result = tokio::select! {
            biased;
            _ = permit.cancelled() => {
                self.metrics.event(event::REQUEST_PREEMPTED, None, "during invoke");
                Err(gate_error_while_generating(permit.cancel_error()))
            }
            result = self.inner.invoke(state, request) => result,
        };
        if let Ok(response) = &result {
            record_usage(&self.metrics, response.usage);
            if loading {
                self.metrics.event(
                    event::MODEL_READY,
                    None,
                    format!(
                        "model={} first response after {}ms",
                        self.model_id,
                        started.elapsed().as_millis()
                    ),
                );
            }
        }
        log::debug!(
            "[mlx:gate] model={} invoke done ok={} in {}ms",
            self.model_id,
            result.is_ok(),
            started.elapsed().as_millis()
        );
        drop(permit);
        result
    }

    async fn stream(&self, state: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        let (permit, loading) = self.admit().await?;
        let inner = tokio::select! {
            biased;
            _ = permit.cancelled() => {
                self.metrics.event(event::REQUEST_PREEMPTED, None, "opening stream");
                return Err(gate_error(permit.cancel_error()));
            }
            opened = self.inner.stream(state, request) => opened?,
        };
        Ok(gated_stream(GatedStream {
            inner,
            permit,
            metrics: Arc::clone(&self.metrics),
            loading,
            model_id: self.model_id.clone(),
            last_usage: None,
        }))
    }
}

/// A stream that owns the gate permit. Dropping it releases the permit.
struct GatedStream {
    inner: ModelStream,
    permit: GatePermit,
    metrics: Arc<MetricsSink>,
    loading: bool,
    model_id: String,
    last_usage: Option<Usage>,
}

fn gated_stream(state: GatedStream) -> ModelStream {
    Box::pin(futures::stream::unfold(Some(state), |state| async move {
        let mut st = state?;
        let next = tokio::select! {
            biased;
            _ = st.permit.cancelled() => {
                st.metrics.event(event::REQUEST_PREEMPTED, None, "during stream");
                // Ending here drops `st`, and with it the permit.
                return Some((
                    ModelStreamItem::Failed(st.permit.cancel_error().message()),
                    None,
                ));
            }
            item = st.inner.next() => item,
        };
        let item = next?;
        let terminal = matches!(
            item,
            ModelStreamItem::Completed(_)
                | ModelStreamItem::Failed(_)
                | ModelStreamItem::ProviderFailed(_)
        );
        match &item {
            ModelStreamItem::UsageDelta(usage) => st.last_usage = Some(*usage),
            ModelStreamItem::Completed(response) => {
                record_usage(&st.metrics, response.usage.or(st.last_usage));
                if st.loading {
                    st.metrics.event(
                        event::MODEL_READY,
                        None,
                        format!("model={} first stream done", st.model_id),
                    );
                }
            }
            _ => {}
        }
        if terminal {
            log::debug!("[mlx:gate] model={} stream finished", st.model_id);
            // Release at the terminal item rather than when the caller gets
            // round to dropping the stream.
            Some((item, None))
        } else {
            Some((item, Some(st)))
        }
    }))
}

/// Put an `mlx:` chat model behind the gate when the MLX supervisor is on.
/// Anything else is returned unchanged.
pub(crate) fn gate_mlx_chat_model(
    chat: Arc<dyn ChatModel<()>>,
    model_id: &str,
    config: &Config,
) -> Arc<dyn ChatModel<()>> {
    if !config.mlx.enabled {
        return chat;
    }
    let svc = super::global(config);
    // An explicit endpoint override points somewhere the supervisor does not
    // own; serialize requests to it but do not start or unload anything.
    let manage = std::env::var("OPENHUMAN_LOCAL_INFERENCE_URL")
        .map(|value| value.trim().is_empty())
        .unwrap_or(true);
    let worker = Arc::new(ServiceWorker {
        svc: Arc::clone(&svc),
        config: Arc::new(config.clone()),
        manage,
    });
    let gated: Arc<dyn ChatModel<()>> = Arc::new(GatedLocalModel::new(
        chat,
        model_id,
        Arc::clone(&svc.gate),
        Arc::clone(&svc.metrics),
        worker,
        config.mlx.worker.clone(),
    ));
    #[cfg(test)]
    test_hook::register(&gated);
    log::debug!("[mlx:gate] gating model={model_id} manage_worker={manage}");
    gated
}

#[cfg(test)]
pub(crate) mod test_hook {
    //! Identify gated models without downcasting (`ChatModel` is not `Any`).
    use std::collections::HashSet;
    use std::sync::Arc;

    use tinyagents::harness::model::ChatModel;

    static GATED: once_cell::sync::Lazy<parking_lot::Mutex<HashSet<usize>>> =
        once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(HashSet::new()));

    fn key(model: &Arc<dyn ChatModel<()>>) -> usize {
        Arc::as_ptr(model).cast::<()>() as usize
    }

    pub(super) fn register(model: &Arc<dyn ChatModel<()>>) {
        GATED.lock().insert(key(model));
    }

    /// Whether `model` came out of `gate_mlx_chat_model` wrapped.
    pub(crate) fn is_gated(model: &Arc<dyn ChatModel<()>>) -> bool {
        GATED.lock().contains(&key(model))
    }
}

#[cfg(test)]
#[path = "gated_model_tests.rs"]
mod tests;
