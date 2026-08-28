//! The device-local identity that stands in for a hosted account.
//!
//! # What replaces "who is signed in"
//!
//! A hosted install answers that question with an account on the backend. A
//! local install has no account and needs none: the app is single-user on this
//! machine, so identity collapses to "this device". The user id is derived from
//! the hostname (`local-my-macbook`) by the same
//! [`local_session_user_id`](crate::openhuman::security::credentials::session_support::local_session_user_id)
//! the offline path already used, so a workspace created before local mode
//! keeps its id and its data.
//!
//! # Why the token looks like a JWT
//!
//! Because everything downstream already expects one. The credentials store,
//! the renderer's session handling, and
//! [`is_local_session_token`](crate::openhuman::security::credentials::session_support::is_local_session_token)
//! were built around the hosted session JWT, and the offline path had already
//! settled on `header.payload.local` — a JWT-shaped triple whose signature
//! segment is the literal string `local`.
//!
//! It is **not signed, and must never be treated as if it were**. There is
//! nothing for a signature to prove: the issuer and the verifier are the same
//! process on the same machine, and a secret stored next to the token it
//! protects buys nothing. The `local` sentinel is what makes that
//! unforgeability claim impossible to make by accident — no verifier can
//! mistake it for a signature it should have checked, the way a random-looking
//! third segment could be.

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::openhuman::security::credentials::session_support::local_session_user_id;

/// The signature segment that marks a token as device-local. Shared with
/// [`is_local_session_token`](crate::openhuman::security::credentials::session_support::is_local_session_token),
/// which is the reader half of this contract.
pub(crate) const LOCAL_TOKEN_MARKER: &str = "local";

/// How long a minted local session claims to be valid. A year: long enough not
/// to be a recurring annoyance, short enough that a token copied out of a
/// backup eventually stops working.
const LOCAL_SESSION_TTL_SECS: i64 = 365 * 24 * 60 * 60;

/// The local user, in the shape `/auth/me` and `/auth/profile` return.
///
/// Field names match the hosted payload (`firstName`, not `first_name`) because
/// the renderer and `api::rest` parse it with the hosted schema — a local
/// install must not need a second parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalIdentity {
    pub id: String,
    #[serde(rename = "_id")]
    pub underscore_id: String,
    pub name: String,
    pub email: String,
    #[serde(rename = "firstName")]
    pub first_name: String,
    #[serde(rename = "lastName")]
    pub last_name: Option<String>,
    pub username: String,
}

impl LocalIdentity {
    /// The identity for this device.
    pub fn for_device() -> Self {
        let id = local_session_user_id();
        Self {
            name: "Local User".to_string(),
            // A syntactically valid address in the reserved `.local` TLD, so
            // nothing downstream that validates an email rejects it and nothing
            // that *sends* to one can reach a real inbox.
            email: format!("{id}@openhuman.local"),
            first_name: "Local".to_string(),
            last_name: None,
            username: id.clone(),
            underscore_id: id.clone(),
            id,
        }
    }
}

fn base64url(value: &serde_json::Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value.to_string())
}

/// Mint a device-local session token for `user_id`.
///
/// `issued_at_unix` is a parameter rather than a `Utc::now()` call so the
/// expiry arithmetic is testable without freezing the clock.
pub fn local_session_token_at(user_id: &str, issued_at_unix: i64) -> String {
    let header = serde_json::json!({ "alg": "none", "typ": "JWT" });
    let payload = serde_json::json!({
        "sub": user_id,
        "user_id": user_id,
        "iat": issued_at_unix,
        "exp": issued_at_unix + LOCAL_SESSION_TTL_SECS,
        // Names the minter so a token found in a keyring or a log is
        // self-describing. Read by nothing — diagnostics only.
        "iss": "openhuman-local-backend",
    });
    format!(
        "{}.{}.{LOCAL_TOKEN_MARKER}",
        base64url(&header),
        base64url(&payload)
    )
}

/// Mint a device-local session token for `user_id`, issued now.
pub fn local_session_token(user_id: &str) -> String {
    local_session_token_at(user_id, chrono::Utc::now().timestamp())
}

/// Is `token` a device-local session token?
///
/// Delegates to the credentials store's reader so the two halves of the
/// contract cannot drift: a token this module mints must be one that module
/// recognises.
pub fn is_local_token(token: &str) -> bool {
    crate::openhuman::security::credentials::session_support::is_local_session_token(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_tokens_are_recognised_as_local() {
        let token = local_session_token("local-test");
        assert!(
            is_local_token(&token),
            "the minter and the credentials store's reader must agree"
        );
    }

    #[test]
    fn token_carries_the_literal_local_marker_not_a_signature() {
        let token = local_session_token("local-test");
        let segments: Vec<&str> = token.split('.').collect();
        assert_eq!(segments.len(), 3);
        assert_eq!(
            segments[2], LOCAL_TOKEN_MARKER,
            "a random-looking third segment could be mistaken for a signature \
             a verifier forgot to check"
        );
    }

    #[test]
    fn payload_round_trips_the_user_id_and_expiry() {
        let token = local_session_token_at("local-box", 1_700_000_000);
        let payload = token.split('.').nth(1).expect("payload segment");
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .expect("payload is base64url");
        let value: serde_json::Value = serde_json::from_slice(&decoded).expect("payload is JSON");

        assert_eq!(value["sub"], "local-box");
        assert_eq!(value["user_id"], "local-box");
        assert_eq!(value["iat"], 1_700_000_000i64);
        assert_eq!(value["exp"], 1_700_000_000i64 + LOCAL_SESSION_TTL_SECS);
    }

    #[test]
    fn expiry_is_readable_by_the_shared_jwt_decoder() {
        // `credentials::ops` records `expires_at` from this at store time; if
        // the payload were unreadable the store would silently record `None`
        // and lose local-session expiry tracking.
        let token = local_session_token_at("local-box", 1_700_000_000);
        let exp = crate::api::jwt::decode_jwt_exp(&token).expect("exp decodes");
        assert_eq!(exp.timestamp(), 1_700_000_000 + LOCAL_SESSION_TTL_SECS);
    }

    #[test]
    fn identity_id_matches_the_credential_stores_local_user_id() {
        // A workspace created on the pre-existing offline path must keep its
        // id, and therefore its data, when local mode is turned on.
        let identity = LocalIdentity::for_device();
        assert_eq!(identity.id, local_session_user_id());
        assert_eq!(identity.underscore_id, identity.id);
        assert_eq!(identity.username, identity.id);
    }

    #[test]
    fn identity_email_is_in_the_reserved_local_tld() {
        let identity = LocalIdentity::for_device();
        assert!(
            identity.email.ends_with("@openhuman.local"),
            "a local identity must not carry an address that could reach a real inbox"
        );
    }

    #[test]
    fn identity_serializes_with_the_hosted_field_names() {
        // `api::rest` and the renderer parse this with the hosted schema.
        let json = serde_json::to_value(LocalIdentity::for_device()).expect("serializes");
        for key in ["id", "_id", "name", "email", "firstName", "username"] {
            assert!(json.get(key).is_some(), "missing hosted field `{key}`");
        }
    }

    #[test]
    fn a_hosted_looking_token_is_not_local() {
        assert!(!is_local_token("header.payload.c2lnbmF0dXJl"));
        assert!(!is_local_token(""));
    }
}
