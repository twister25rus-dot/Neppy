//! Shared state for the local backend's handlers.

use std::sync::Arc;
use std::time::SystemTime;

use super::identity::LocalIdentity;

/// Handler state. Cheap to clone — every field is either `Copy` or behind an
/// `Arc`, because axum clones this per request.
#[derive(Clone)]
pub struct LocalBackendState {
    /// The device identity `/auth/*` reports.
    pub identity: Arc<LocalIdentity>,
    /// Bearer accepted on every guarded route, minted at startup.
    pub session_token: Arc<String>,
    /// Core version, reported by `/version` and the client-version headers.
    pub core_version: &'static str,
    /// When the server bound, for `/health` uptime.
    pub started_at: SystemTime,
    /// Whether `/openai/v1/*` forwards to the local runtime or answers
    /// unsupported. Mirrors `local_mode.proxy_inference`.
    pub proxy_inference: bool,
}

impl LocalBackendState {
    pub fn new(proxy_inference: bool) -> Self {
        let identity = LocalIdentity::for_device();
        let session_token = super::identity::local_session_token(&identity.id);
        Self {
            identity: Arc::new(identity),
            session_token: Arc::new(session_token),
            core_version: env!("CARGO_PKG_VERSION"),
            started_at: SystemTime::now(),
            proxy_inference,
        }
    }

    /// Seconds since the server bound. Saturates at zero rather than panicking
    /// if the wall clock moves backwards mid-run (NTP step, suspend/resume) —
    /// an uptime readout is not worth taking the process down for.
    pub fn uptime_secs(&self) -> u64 {
        self.started_at.elapsed().map(|d| d.as_secs()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn state_mints_a_token_for_its_own_identity() {
        let state = LocalBackendState::new(true);
        assert!(super::super::identity::is_local_token(&state.session_token));
        let payload = state.session_token.split('.').nth(1).unwrap();
        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, payload)
                .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(value["sub"], state.identity.id.as_str());
    }

    #[test]
    fn uptime_does_not_panic_on_a_backwards_clock() {
        let mut state = LocalBackendState::new(true);
        state.started_at = SystemTime::now() + std::time::Duration::from_secs(3600);
        assert_eq!(state.uptime_secs(), 0);
    }

    #[test]
    fn proxy_inference_flag_is_carried_through() {
        assert!(LocalBackendState::new(true).proxy_inference);
        assert!(!LocalBackendState::new(false).proxy_inference);
    }
}
