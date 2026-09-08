//! Health probing and the per-server state machine.
//!
//! Two endpoints matter. `/v1/models` is the portable one — both binaries
//! serve it, it is what the provider factory will talk to, and a successful
//! response proves the OpenAI surface is actually up rather than merely that a
//! port is bound. `mlx_vlm.server` additionally serves `/health`, which is
//! cheaper because it does not enumerate the Hugging Face cache; with
//! `model_discovery = "hf-cache"` a `/v1/models` call scans the cache
//! directory, so polling it every second during startup is wasteful.
//!
//! Startup is slow by nature: loading a 27B checkpoint takes tens of seconds,
//! during which the port is bound but requests hang. `Starting` is therefore a
//! real state with a generous ceiling, not a brief transition.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Lifecycle state of one managed server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MlxServerState {
    /// Not running, and not asked to be.
    Stopped,
    /// Process is up; the HTTP surface has not answered yet. Normal while a
    /// large checkpoint loads.
    Starting,
    /// Answering on `/v1/models`.
    Ready,
    /// Process is alive but the HTTP surface stopped answering.
    Degraded,
    /// Process exited on its own.
    Crashed,
}

impl MlxServerState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Crashed => "crashed",
        }
    }

    /// Whether the server can serve inference right now.
    pub(crate) fn is_usable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Outcome of one probe.
#[derive(Debug, Clone)]
pub(crate) struct HealthReport {
    pub(crate) reachable: bool,
    /// Model ids the server advertises. With `model_discovery = "hf-cache"`
    /// this includes cached models that are not loaded yet.
    pub(crate) models: Vec<String>,
    /// Why the probe failed, when it did.
    pub(crate) detail: Option<String>,
}

impl HealthReport {
    fn unreachable(detail: impl Into<String>) -> Self {
        Self {
            reachable: false,
            models: Vec::new(),
            detail: Some(detail.into()),
        }
    }
}

/// Shape of an OpenAI-compatible `/v1/models` response. Only the ids are read;
/// `#[serde(default)]` keeps an unexpected payload from failing the probe.
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    #[serde(default)]
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    #[serde(default)]
    id: String,
}

/// `mlx_vlm.server`'s `/health` payload.
///
/// Richer than a status code, and the reason liveness is worth probing
/// separately: it names which checkpoints are actually resident, which is what
/// the status UI needs and what `/v1/models` cannot say — under
/// `model_discovery = "hf-cache"` that endpoint lists the whole cache, loaded
/// or not.
///
/// Every field is optional. `mlx_lm.server` has no `/health` at all, and the
/// payload has grown across releases, so a missing key must degrade rather
/// than fail the probe.
#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct LivenessReport {
    /// Checkpoint currently serving `/v1/chat/completions`, if any.
    #[serde(default)]
    pub(crate) loaded_model: Option<String>,
    /// Every resident slot, keyed by role. This is the multi-slot process
    /// reporting what it actually holds in memory.
    #[serde(default)]
    pub(crate) loaded_models: std::collections::HashMap<String, serde_json::Value>,
    /// Context window the loaded model was opened with.
    #[serde(default)]
    pub(crate) loaded_context_size: Option<u32>,
    /// Ceiling the server will enforce, after its own clamping.
    #[serde(default)]
    pub(crate) effective_context_limit: Option<u32>,
    /// Tool-call parser selected for the loaded model. `None` with a model
    /// loaded means tool calls will not be parsed out of the reply.
    #[serde(default)]
    pub(crate) loaded_tool_parser: Option<String>,
    #[serde(default)]
    pub(crate) continuous_batching_enabled: bool,
}

impl LivenessReport {
    /// Whether any model slot is resident.
    pub(crate) fn has_resident_model(&self) -> bool {
        self.loaded_model.is_some() || !self.loaded_models.is_empty()
    }
}

/// Probe the OpenAI surface at `base_url` (which ends in `/v1`).
pub(crate) async fn probe_models(
    client: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
) -> HealthReport {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut request = client.get(&url).timeout(Duration::from_secs(5));
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }

    tracing::debug!(
        target: "local_ai::mlx_admin",
        %url,
        "[mlx] probing model list"
    );

    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => return HealthReport::unreachable(format!("request failed: {err}")),
    };

    let status = response.status();
    if !status.is_success() {
        // 401 here means the server has an api-key the client is not sending,
        // or the reverse. Say so rather than reporting a generic outage.
        let hint = if status.as_u16() == 401 {
            " (the server requires a bearer token — check auth and api_key)"
        } else {
            ""
        };
        return HealthReport::unreachable(format!("HTTP {status}{hint}"));
    }

    match response.json::<ModelsResponse>().await {
        Ok(parsed) => HealthReport {
            reachable: true,
            models: parsed
                .data
                .into_iter()
                .map(|entry| entry.id)
                .filter(|id| !id.is_empty())
                .collect(),
            detail: None,
        },
        // A 200 that will not parse still proves the server is listening and
        // authenticated, so treat it as reachable with an empty catalogue
        // rather than reporting it down.
        Err(err) => HealthReport {
            reachable: true,
            models: Vec::new(),
            detail: Some(format!("model list could not be parsed: {err}")),
        },
    }
}

/// Cheap liveness check, `mlx_vlm.server` only.
///
/// Derived from the `/v1` base by trimming the suffix, so callers hold one URL.
pub(crate) async fn probe_liveness(
    client: &reqwest::Client,
    base_url: &str,
) -> Option<LivenessReport> {
    let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
    let response = client
        .get(format!("{root}/health"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    // A 200 that will not parse still proves liveness, so fall back to an
    // empty report rather than reporting the server down.
    Some(response.json::<LivenessReport>().await.unwrap_or_default())
}

/// Classify a server from a probe plus whether its process is still alive.
///
/// The exit status is the deciding input: a server whose HTTP surface is not
/// answering is `Starting` while the process lives and `Crashed` once it does
/// not, and those two want very different UI. Passing `has_been_ready` keeps a
/// server that answered once and then stopped from being reported as still
/// starting up.
pub(crate) fn classify(
    report: &HealthReport,
    process_alive: bool,
    has_been_ready: bool,
) -> MlxServerState {
    if !process_alive {
        return MlxServerState::Crashed;
    }
    if report.reachable {
        return MlxServerState::Ready;
    }
    if has_been_ready {
        MlxServerState::Degraded
    } else {
        MlxServerState::Starting
    }
}
