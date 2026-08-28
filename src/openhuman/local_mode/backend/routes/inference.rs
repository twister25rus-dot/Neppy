//! `/openai/v1/*` — the OpenAI-compatible surface, served locally.
//!
//! # Chat completions and models
//!
//! These are **not** reimplemented. The core already exposes an
//! OpenAI-compatible `/v1/chat/completions` and `/v1/models`
//! ([`inference::http`](crate::openhuman::inference::http)) that routes through
//! the same unified `Provider` trait the agent uses, so this module mounts that
//! exact router under `/openai/v1`. One implementation, one set of provider
//! semantics, and a fix to either surface fixes both.
//!
//! ## Why this cannot loop back on itself
//!
//! In local mode `effective_backend_api_url` points here, so a workload routed
//! to the *managed* provider would dial this endpoint, whose handler resolves a
//! provider — managed again. The cycle is cut in the provider factory, not
//! here: `enforce_local_mode_inference` refuses to construct the managed
//! provider under local mode and returns an error naming the local runtimes.
//! Cutting it at construction rather than at the HTTP boundary means the CLI
//! and the agent loop get the same clear error, not a connection storm.
//!
//! # Embeddings
//!
//! `/v1/embeddings` has no counterpart in the core's router — the embedding
//! stack is reached through the memory subsystem rather than over HTTP — so it
//! is implemented here against the configured [`EmbeddingProvider`]. The
//! response is the OpenAI embeddings shape because that is what the clients
//! calling this path already parse.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::core::types::AppState;
use crate::openhuman::config::Config;
use crate::openhuman::local_mode::backend::state::LocalBackendState;
use crate::openhuman::local_mode::backend::LOG_PREFIX;

/// Mount the OpenAI-compatible surface under `/openai/v1`.
///
/// The chat/models sub-router carries its own [`AppState`], so it is nested as
/// a service — the outer router's state is [`LocalBackendState`] and axum will
/// not nest a differently-stated `Router` any other way.
pub(crate) fn router() -> Router<LocalBackendState> {
    let core_inference = crate::openhuman::inference::http::router().with_state(AppState {
        core_version: env!("CARGO_PKG_VERSION").to_string(),
    });

    Router::new()
        .route("/openai/v1/embeddings", post(embeddings))
        .nest_service("/openai/v1", core_inference)
}

/// OpenAI embeddings request. `input` accepts a bare string or an array, which
/// is what the spec allows and what real clients send.
#[derive(Debug, Deserialize)]
struct EmbeddingsRequest {
    #[serde(default)]
    input: EmbeddingsInput,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
enum EmbeddingsInput {
    Single(String),
    Many(Vec<String>),
    #[default]
    Empty,
}

impl EmbeddingsInput {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(text) => vec![text],
            Self::Many(texts) => texts,
            Self::Empty => Vec::new(),
        }
    }
}

fn error_response(status: StatusCode, message: String, kind: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "message": message, "type": kind } })),
    )
        .into_response()
}

/// `POST /openai/v1/embeddings`
async fn embeddings(
    State(_state): State<LocalBackendState>,
    Json(request): Json<EmbeddingsRequest>,
) -> Response {
    let inputs = request.input.into_vec();
    if inputs.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "`input` must be a non-empty string or array of strings".to_string(),
            "invalid_request_error",
        );
    }

    let config = match Config::load_or_init().await {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!("{LOG_PREFIX} embeddings: config load failed: {error}");
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("config load failed: {error}"),
                "internal_error",
            );
        }
    };

    let provider =
        crate::openhuman::inference::embeddings::default_embedding_provider_with_config(&config);

    let borrowed: Vec<&str> = inputs.iter().map(String::as_str).collect();
    let vectors = match provider.embed(&borrowed).await {
        Ok(vectors) => vectors,
        Err(error) => {
            // The common failure here is "no local embedding model pulled",
            // which is a setup problem the user can fix — surface the
            // provider's own message rather than a generic 500 text.
            tracing::warn!("{LOG_PREFIX} embeddings: provider failed: {error:#}");
            return error_response(
                StatusCode::BAD_GATEWAY,
                format!("local embedding provider failed: {error:#}"),
                "provider_error",
            );
        }
    };

    let data: Vec<serde_json::Value> = vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| {
            json!({ "object": "embedding", "index": index, "embedding": embedding })
        })
        .collect();

    Json(json!({
        "object": "list",
        "data": data,
        // Echo the requested model when the caller named one, so a client that
        // asserts on the round-trip is satisfied; otherwise report what
        // actually produced the vectors.
        "model": request.model.unwrap_or_else(|| provider.model_id().to_string()),
        "usage": { "prompt_tokens": 0, "total_tokens": 0 },
    }))
    .into_response()
}
