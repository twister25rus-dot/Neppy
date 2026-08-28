//! `/auth/*` — the device-local session.
//!
//! The hosted routes these stand in for exchange OAuth/magic-link flows for a
//! session JWT. Locally there is no exchange to perform: the session already
//! exists, minted at startup for this device, so every route here reports it
//! rather than negotiating it.
//!
//! `/auth/refresh` is the interesting one. A hosted refresh rotates a
//! short-lived token; locally the token is valid for a year and rotating it
//! would invalidate the copy the renderer holds for no benefit. It therefore
//! returns the *same* token — a successful no-op refresh, which is what every
//! caller's retry-on-401 path expects, rather than a `501` that would surface
//! as a spurious sign-out.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::openhuman::local_mode::backend::state::LocalBackendState;

pub(crate) fn router() -> Router<LocalBackendState> {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/profile", get(profile))
        .route("/auth/refresh", post(refresh))
        .route("/auth/login-token/consume", post(consume_login_token))
}

/// `GET /auth/me` — the hosted shape is `{ user: {...} }`.
async fn me(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({ "user": &*state.identity }))
}

/// `GET /auth/profile` — the hosted shape returns the user at the top level.
///
/// The duplication with `/auth/me` is the hosted API's, not ours; both are
/// live client call sites (`user_id_from_auth_me_payload` and
/// `user_id_from_profile_payload` in `api::rest` parse them separately), so
/// both are served with the shape their parser expects.
async fn profile(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({
        "user": &*state.identity,
        "id": state.identity.id,
        "email": state.identity.email,
        "name": state.identity.name,
    }))
}

/// `POST /auth/refresh` — returns the existing token unchanged.
async fn refresh(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({
        "token": &*state.session_token,
        "jwtToken": &*state.session_token,
        "user": &*state.identity,
    }))
}

/// `POST /auth/login-token/consume` — hands back the device session.
///
/// The hosted route trades a one-time login token (from a browser OAuth hop)
/// for a session JWT. There is no browser hop locally, so any presented token
/// is ignored and the device session is returned. That is not a security
/// downgrade: the caller already passed the bearer guard, which is a stronger
/// check than the one-time token this route would have validated.
async fn consume_login_token(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({
        "result": { "jwtToken": &*state.session_token },
        "token": &*state.session_token,
        "user": &*state.identity,
    }))
}
