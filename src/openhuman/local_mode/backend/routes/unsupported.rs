//! The fallback: every hosted route with no local implementation.
//!
//! Mounted as the router's `fallback`, so it catches the whole hosted surface
//! that is not explicitly served — `/agent-integrations/*`, `/payments/*`,
//! `/referral/*`, `/invites/*`, `/channels/*`, `/voice-agent/*`, and anything a
//! future backend release adds.
//!
//! Being the fallback rather than an enumerated list is deliberate. An
//! enumerated list of unsupported routes would go stale the first time the
//! hosted API grows a route, and the failure mode of a stale list is a bare
//! 404 with no explanation — exactly the outcome this module exists to
//! prevent.

use axum::extract::Request;
use axum::response::{IntoResponse, Response};

use crate::openhuman::local_mode::backend::error::LocalModeError;
use crate::openhuman::local_mode::backend::LOG_PREFIX;

/// Answer any unserved route with the structured `local_mode_unsupported`
/// error for whichever service owns it.
pub(crate) async fn fallback(request: Request) -> Response {
    let path = request.uri().path();
    tracing::info!(
        method = %request.method(),
        path = %path,
        "{LOG_PREFIX} unsupported hosted route — answering local_mode_unsupported"
    );
    LocalModeError::for_path(path).into_response()
}
