//! The structured error a hosted route returns when it has no local
//! implementation.
//!
//! # Why `501` and not `404`
//!
//! A `404` says "this route does not exist", which is false and unhelpful: the
//! route exists, the *service behind it* does not. `501 Not Implemented` is the
//! accurate status, and it is one clients already treat as terminal — a
//! retry-on-404 loop is common, a retry-on-501 loop is not.
//!
//! # Why not a plausible empty success
//!
//! Returning `200 {"results": []}` for a search route, or `200 {"credits": 0}`
//! for billing, would keep more of the UI quiet. It would also be a lie the
//! agent cannot detect: an empty search result is indistinguishable from "the
//! web had nothing", so the model concludes the latter and reasons on. Every
//! unsupported route therefore fails loudly and names the alternative.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::openhuman::local_mode::services::{self, LocalReplacementKind};

/// The machine-readable error code every unsupported-route body carries.
/// Clients branch on this rather than on the message text.
pub const LOCAL_MODE_UNSUPPORTED_CODE: &str = "local_mode_unsupported";

/// Body of a `local_mode_unsupported` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModeErrorBody {
    /// Always [`LOCAL_MODE_UNSUPPORTED_CODE`]. Owned rather than `&'static str`
    /// so the body stays `Deserialize` — clients and tests parse this shape.
    pub code: String,
    /// Human-readable summary, safe to show verbatim in the UI.
    pub error: String,
    /// The requested path, so a client that batches calls can tell which one
    /// failed.
    pub path: String,
    /// Id of the [`services`] entry that owns this route, when one claims it.
    /// The Settings UI joins on this to deep-link the relevant row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    /// The local alternative, in one line.
    pub local_alternative: String,
    /// What the user must do, when the alternative needs setting up. Absent
    /// when there is nothing to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
}

/// An unsupported hosted route, as an axum response.
#[derive(Debug, Clone)]
pub struct LocalModeError(pub LocalModeErrorBody);

impl LocalModeError {
    /// Build the error for `path`, attaching the owning service entry when one
    /// claims the route.
    ///
    /// A path no entry claims still gets a usable answer rather than a bare
    /// 501: the generic text names local mode as the cause, which is the one
    /// thing the caller needs to know to act.
    pub fn for_path(path: &str) -> Self {
        match services::entry_for_path(path) {
            Some(entry) => Self(LocalModeErrorBody {
                code: LOCAL_MODE_UNSUPPORTED_CODE.to_string(),
                error: format!(
                    "`{}` is served by the hosted backend, which local mode does not use. {}",
                    entry.hosted,
                    match entry.kind {
                        LocalReplacementKind::NotApplicable =>
                            "It has no local equivalent because it only exists as part of a \
                             hosted account.",
                        _ => "Use the local alternative instead.",
                    }
                ),
                path: path.to_string(),
                service_id: Some(entry.id.to_string()),
                local_alternative: entry.local_alternative.to_string(),
                setup: (!entry.setup.is_empty()).then(|| entry.setup.to_string()),
            }),
            None => Self(LocalModeErrorBody {
                code: LOCAL_MODE_UNSUPPORTED_CODE.to_string(),
                error: format!("`{path}` is a hosted-backend route with no local implementation."),
                path: path.to_string(),
                service_id: None,
                local_alternative:
                    "Turn local mode off in Settings → Privacy to reach the hosted backend, \
                     or use a local tool or MCP server for this capability."
                        .to_string(),
                setup: None,
            }),
        }
    }
}

impl IntoResponse for LocalModeError {
    fn into_response(self) -> Response {
        (StatusCode::NOT_IMPLEMENTED, Json(self.0)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claimed_route_carries_its_service_entry() {
        let err = LocalModeError::for_path("/agent-integrations/composio/execute");
        assert_eq!(err.0.code, LOCAL_MODE_UNSUPPORTED_CODE);
        assert_eq!(err.0.service_id.as_deref(), Some("integrations.composio"));
        assert_eq!(err.0.path, "/agent-integrations/composio/execute");
        assert!(err.0.local_alternative.contains("MCP"));
    }

    #[test]
    fn a_not_applicable_route_says_so_rather_than_offering_a_substitute() {
        let err = LocalModeError::for_path("/payments/stripe/currentPlan");
        assert_eq!(err.0.service_id.as_deref(), Some("account.billing"));
        assert!(
            err.0.error.contains("no local equivalent"),
            "billing is absent, not degraded — the message must not imply a substitute exists"
        );
    }

    #[test]
    fn setup_text_rides_along_only_when_there_is_something_to_do() {
        let needs_setup = LocalModeError::for_path("/search");
        assert!(needs_setup.0.setup.is_some());

        let nothing_to_do = LocalModeError::for_path("/payments/credits/balance");
        assert!(nothing_to_do.0.setup.is_none());
    }

    #[test]
    fn an_unclaimed_route_still_names_local_mode_as_the_cause() {
        let err = LocalModeError::for_path("/some/unmapped/route");
        assert!(err.0.service_id.is_none());
        assert!(err.0.error.contains("/some/unmapped/route"));
        assert!(err.0.local_alternative.contains("local mode"));
    }

    #[test]
    fn body_serializes_without_the_absent_optionals() {
        let json = serde_json::to_value(LocalModeError::for_path("/payments").0).unwrap();
        assert!(
            json.get("setup").is_none(),
            "absent setup must not serialize as null"
        );
        assert_eq!(json["code"], LOCAL_MODE_UNSUPPORTED_CODE);
    }

    #[test]
    fn responds_501_not_404() {
        // 404 would say the route does not exist, which is false and invites a
        // retry loop. 501 is terminal and accurate.
        let response = LocalModeError::for_path("/payments").into_response();
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
