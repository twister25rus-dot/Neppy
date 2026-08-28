//! Request guard for the local backend.
//!
//! # Why a loopback service still needs one
//!
//! Loopback keeps the service off the network. It does not keep it away from
//! the browser: a page on any site the user visits can issue a cross-origin
//! `POST http://127.0.0.1:43117/...`. Without a guard that page could read
//! `/auth/me` or drive the inference proxy — which spends the user's own
//! provider key.
//!
//! # The two checks, and why neither suffices alone
//!
//! **Origin.** Enforced against the same allow-list the core's RPC server uses
//! ([`is_origin_allowed`](crate::core::jsonrpc::is_origin_allowed)): the Tauri
//! webview and loopback origins, plus anything an operator opted in through
//! `OPENHUMAN_CORE_ALLOWED_ORIGINS`. A request with no `Origin` header is
//! allowed, because browsers always set one on a cross-origin request while
//! non-browser callers (the core itself, curl, a CLI) do not.
//!
//! This is the check that actually stops the browser attack, and it has to be
//! here rather than left to CORS: CORS only stops the page from *reading* the
//! response, and a POST to the inference proxy does its damage on the way in.
//!
//! **Bearer.** One of:
//!
//! * the per-launch core RPC token — in-process callers already hold it, and
//!   requiring a second credential of them would buy nothing; or
//! * a device-local session token (`header.payload.local`).
//!
//! The second arm is deliberately a *shape* check, not an equality check
//! against the token this process minted, and it is worth being explicit about
//! why: the token is unsigned by design (see
//! [`identity`](super::identity) — the issuer and the verifier are the same
//! process on the same machine), its format is in the open-source tree, and the
//! renderer mints its own on the "Continue locally" path. An equality check
//! would therefore reject the app's own frontend while stopping no attacker who
//! had read the source. Pretending otherwise would be *worse* than not
//! checking: it would put a security claim on a comparison that does not carry
//! one, and invite the origin check to be dropped as redundant.
//!
//! So the bearer arm is what it honestly is — a sanity check that the caller is
//! speaking this protocol — and the origin check is what carries the security
//! weight. The core-token arm keeps its constant-time comparison because that
//! token *is* a real secret.
//!
//! `/health` and `/version` are exempt from the bearer: they carry no user data
//! and are what a supervisor polls before a token exists. They are **not**
//! exempt from the origin check.

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::identity::is_local_token;
use super::state::LocalBackendState;
use super::LOG_PREFIX;

/// Routes reachable without a bearer.
const PUBLIC_PATHS: &[&str] = &["/health", "/version"];

/// Is `path` exempt from the bearer requirement?
///
/// Exact match only. A prefix test would exempt `/health/../auth/me` on a
/// server that normalizes lazily, and every public route here is a leaf.
pub(crate) fn is_public_path(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
}

/// Extract a bearer token from an `Authorization` header value.
///
/// The scheme match is case-insensitive per RFC 7235; the token is returned
/// with surrounding whitespace trimmed.
pub(crate) fn bearer_from_header(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

/// Does `presented` satisfy the bearer arm?
///
/// Pure over its inputs so the truth table is testable without a live server or
/// the process-global core token. See the module docs for what each arm is
/// worth.
pub(crate) fn token_is_accepted(presented: &str, core_token: Option<&str>) -> bool {
    if core_token.is_some_and(|core| crate::core::auth::bearer_matches(presented, core)) {
        return true;
    }
    is_local_token(presented)
}

/// Does the request's `Origin` header permit it?
///
/// `None` (no header) is permitted: browsers always set `Origin` on a
/// cross-origin request, so its absence means a non-browser caller.
pub(crate) fn origin_is_allowed(origin: Option<&str>) -> bool {
    origin.is_none_or(crate::core::jsonrpc::is_origin_allowed)
}

fn refuse(status: StatusCode, code: &'static str, error: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({ "code": code, "error": error })),
    )
        .into_response()
}

/// axum middleware enforcing the guard.
pub(crate) async fn require_bearer(
    // Unused, but `from_fn_with_state` requires the state extractor. The
    // minted session token lives on it for the routes that *return* it
    // (`/auth/refresh`, `/auth/login-token/consume`); nothing here compares
    // against it — see the module docs.
    State(_state): State<LocalBackendState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();

    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    if !origin_is_allowed(origin.as_deref()) {
        // Log the origin — it is the actionable part of the diagnostic and is
        // not a secret. Never log the presented bearer.
        tracing::warn!(
            origin = origin.as_deref().unwrap_or(""),
            "{LOG_PREFIX} 403 path={path} (origin not allowed)"
        );
        return refuse(
            StatusCode::FORBIDDEN,
            "forbidden_origin",
            "The local backend does not accept requests from this origin.",
        );
    }

    if is_public_path(&path) {
        return next.run(request).await;
    }

    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_from_header);

    let core_token = crate::core::auth::get_rpc_token();
    let accepted = presented.is_some_and(|token| token_is_accepted(token, core_token));

    if !accepted {
        tracing::warn!("{LOG_PREFIX} 401 path={path} (no acceptable bearer)");
        return refuse(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "The local backend requires a device session bearer.",
        );
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_paths_are_matched_exactly() {
        assert!(is_public_path("/health"));
        assert!(is_public_path("/version"));
        assert!(!is_public_path("/auth/me"));
        // A prefix test would exempt this on a lazily-normalizing server.
        assert!(!is_public_path("/health/../auth/me"));
        assert!(!is_public_path("/healthz"));
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(bearer_from_header("Bearer abc"), Some("abc"));
        assert_eq!(bearer_from_header("bearer abc"), Some("abc"));
        assert_eq!(bearer_from_header("BEARER abc"), Some("abc"));
    }

    #[test]
    fn a_non_bearer_scheme_yields_nothing() {
        assert_eq!(bearer_from_header("Basic abc"), None);
        assert_eq!(bearer_from_header("abc"), None);
        assert_eq!(bearer_from_header(""), None);
    }

    #[test]
    fn an_empty_bearer_value_yields_nothing() {
        assert_eq!(bearer_from_header("Bearer "), None);
        assert_eq!(bearer_from_header("Bearer    "), None);
    }

    #[test]
    fn the_core_rpc_token_is_accepted() {
        assert!(token_is_accepted("core-secret", Some("core-secret")));
    }

    #[test]
    fn any_device_local_token_is_accepted() {
        // The renderer mints its own on the "Continue locally" path, so an
        // equality check against this process's token would reject the app's
        // own frontend. See the module docs for why this is honest rather than
        // lax: the origin check is what carries the security weight.
        assert!(token_is_accepted("header.payload.local", None));
        assert!(token_is_accepted(
            &super::super::identity::local_session_token("local-box"),
            None
        ));
    }

    #[test]
    fn a_hosted_shaped_or_empty_bearer_is_not_accepted() {
        assert!(!token_is_accepted("header.payload.c2ln", Some("core")));
        assert!(!token_is_accepted("", Some("core")));
        assert!(!token_is_accepted("nonsense", None));
        // Near-misses on the core token must not slip through the equality arm.
        assert!(!token_is_accepted("core-secre", Some("core-secret")));
        assert!(!token_is_accepted("core-secrett", Some("core-secret")));
    }

    #[test]
    fn a_request_without_an_origin_is_allowed() {
        // Browsers always set Origin on a cross-origin request; its absence
        // means the core itself, a CLI, or curl.
        assert!(origin_is_allowed(None));
    }

    #[test]
    fn the_tauri_and_loopback_origins_are_allowed() {
        assert!(origin_is_allowed(Some("tauri://localhost")));
        assert!(origin_is_allowed(Some("http://localhost:1420")));
        assert!(origin_is_allowed(Some("http://127.0.0.1:5173")));
    }

    #[test]
    fn a_website_origin_is_refused() {
        // The attack this guard exists for: a page the user visited issuing a
        // cross-origin POST to the inference proxy.
        assert!(!origin_is_allowed(Some("https://evil.example")));
        assert!(!origin_is_allowed(Some("http://evil.example")));
    }
}
