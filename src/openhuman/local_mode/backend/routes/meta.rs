//! Health, version, announcements and the telemetry sink.
//!
//! Small routes whose only job is to answer the way a hosted backend would so
//! the client's normal path runs to completion.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::openhuman::local_mode::backend::state::LocalBackendState;

pub(crate) fn router() -> Router<LocalBackendState> {
    Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/announcements/latest", get(latest_announcement))
        .route("/telemetry/langfuse/ingestion", post(telemetry_sink))
}

/// `GET /health` — public, so a supervisor can poll before a token exists.
async fn health(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "mode": "local",
        "version": state.core_version,
        "uptimeSecs": state.uptime_secs(),
    }))
}

/// `GET /version` — public. Reports the running core, and says plainly that
/// there is no hosted minimum to satisfy.
///
/// The hosted route drives the "please update" gate. A local install has no
/// server that could require an update, so `minSupported` mirrors the running
/// version: the gate evaluates, passes, and stays out of the way — which is
/// what a client with a hardcoded comparison needs. Returning `null` would make
/// a client that parses it strictly throw instead.
async fn version(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({
        "version": state.core_version,
        "minSupported": state.core_version,
        "minSupportedVersion": state.core_version,
        "updateRequired": false,
        "source": "local",
    }))
}

/// `GET /announcements/latest`
///
/// Announcements are product messages from the hosted service. There are none,
/// and there is no mechanism by which there could be. `204 No Content` says
/// that precisely — an empty-object `200` would leave clients that check for
/// presence rather than content rendering an empty banner.
async fn latest_announcement() -> impl IntoResponse {
    StatusCode::NO_CONTENT
}

/// `POST /telemetry/langfuse/ingestion`
///
/// Accepts and drops. The hosted route forwards trace batches to the project's
/// Langfuse; local mode has nowhere off-device to forward them and would not
/// send them there if it did.
///
/// Answering `202 Accepted` rather than `501` is deliberate: trace export is a
/// fire-and-forget side channel, and a failing exporter makes real noise
/// (retry loops, error logs, and in the worst case backpressure on the agent
/// loop) in exchange for telling the user something they already chose. The
/// batch is counted in the log so the drop is observable.
async fn telemetry_sink(body: Option<Json<serde_json::Value>>) -> impl IntoResponse {
    let batch_size = body
        .as_ref()
        .and_then(|Json(value)| value.get("batch"))
        .and_then(|batch| batch.as_array())
        .map(|batch| batch.len())
        .unwrap_or(0);
    tracing::debug!(
        batch_size,
        "{} telemetry batch dropped — tracing is off-device by definition in local mode",
        crate::openhuman::local_mode::backend::LOG_PREFIX
    );
    (
        StatusCode::ACCEPTED,
        Json(json!({ "successes": [], "errors": [] })),
    )
}
