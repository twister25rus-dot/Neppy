//! Local Mode configuration — the *service topology* the app runs under.
//!
//! This is DISTINCT from [`PrivacyMode`](super::PrivacyMode). Privacy mode
//! governs how much user **data** may leave the device; local mode governs
//! **which services the app depends on**. The two are orthogonal:
//!
//! | | `privacy.mode` | `local_mode.enabled` |
//! |---|---|---|
//! | Default install | `standard` | `false` — hosted backend serves auth, inference, search |
//! | Local-only inference, hosted control plane | `local_only` | `false` |
//! | Self-hosted install, cloud LLM on the user's own key | `standard` | `true` |
//! | Fully offline | `local_only` | `true` |
//!
//! Local mode exists because `PrivacyMode::LocalOnly` deliberately exempts the
//! backend **control plane** — see
//! [`is_control_plane`](crate::openhuman::security::egress). That exemption is
//! correct for a privacy posture (blocking sign-in buys no privacy), but it is
//! exactly what stops the app from running without
//! `api.tinyhumans.ai`. Local mode closes that hole: the control plane is
//! served by the in-process [local backend]
//! (crate::openhuman::local_mode::backend) on loopback instead.
//!
//! ## What turning it on does
//!
//! 1. [`effective_backend_api_url`](crate::api::config::effective_backend_api_url)
//!    resolves to the loopback local backend rather than the hosted API.
//! 2. The egress spine refuses any hosted-backend round-trip, control plane
//!    included (`local_mode` arm of
//!    [`local_only_blocks`](crate::openhuman::security::egress::local_only_blocks)).
//! 3. Subsystems whose default is "managed" resolve to their local equivalent
//!    — see [`crate::openhuman::local_mode::services`] for the full inventory.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Runtime env key that force-enables local mode without editing `config.toml`.
///
/// Accepts the usual truthy set (`1` / `true` / `yes` / `on`, case-insensitive);
/// anything else — including an explicit `0` — leaves the config value in
/// charge. Read by [`crate::openhuman::local_mode::is_local_mode`].
pub const LOCAL_MODE_ENV_VAR: &str = "OPENHUMAN_LOCAL_MODE";

/// Env key overriding the TCP port the local backend binds on loopback.
pub const LOCAL_BACKEND_PORT_ENV_VAR: &str = "OPENHUMAN_LOCAL_BACKEND_PORT";

/// Default loopback port for the local backend.
///
/// Deliberately **not** one of
/// [`LOCAL_AI_PORTS`](crate::api::config) (11434 / 8000 / 8080 / 1234 / 8888):
/// `effective_backend_api_url` classifies a loopback override on one of those
/// as a local *model runner* and falls back to the hosted default chain, which
/// would silently undo local mode. Picked from the IANA dynamic range and
/// unclaimed by any tool the project bundles.
pub const DEFAULT_LOCAL_BACKEND_PORT: u16 = 43117;

/// The `[local_mode]` config block.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LocalModeConfig {
    /// Run without the project's hosted backend. Missing key → `false`, so an
    /// existing install keeps its hosted control plane until the user opts in.
    pub enabled: bool,

    /// Loopback port for the in-process local backend. `0` asks the OS for an
    /// ephemeral port, which the server then publishes through
    /// [`crate::openhuman::local_mode::local_backend_base_url`].
    pub backend_port: u16,

    /// Apply local defaults to subsystems that would otherwise resolve to a
    /// managed service (embeddings → Ollama, search → SearXNG, agent tracing →
    /// off). Turn this off to keep local mode's *topology* change while
    /// choosing every provider by hand.
    pub apply_local_defaults: bool,

    /// Serve `/openai/v1/*` from the local backend by forwarding to the
    /// configured local model runtime. Turn this off when `inference_url`
    /// already points somewhere the app should call directly.
    pub proxy_inference: bool,
}

impl Default for LocalModeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            backend_port: DEFAULT_LOCAL_BACKEND_PORT,
            apply_local_defaults: true,
            proxy_inference: true,
        }
    }
}

impl LocalModeConfig {
    /// Port to bind, clamped to something the OS will accept. `0` is preserved
    /// (ephemeral-port request); every other value passes through.
    pub fn effective_backend_port(&self) -> u16 {
        self.backend_port
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_with_the_reserved_port() {
        let cfg = LocalModeConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.backend_port, DEFAULT_LOCAL_BACKEND_PORT);
        assert!(cfg.apply_local_defaults);
        assert!(cfg.proxy_inference);
    }

    #[test]
    fn missing_block_deserializes_to_the_default() {
        #[derive(serde::Deserialize)]
        struct Fragment {
            #[serde(default)]
            local_mode: LocalModeConfig,
        }
        let parsed: Fragment = toml::from_str("").expect("empty toml deserializes");
        assert!(!parsed.local_mode.enabled);
        assert_eq!(parsed.local_mode.backend_port, DEFAULT_LOCAL_BACKEND_PORT);
    }

    #[test]
    fn partial_block_keeps_the_other_defaults() {
        let parsed: LocalModeConfig =
            toml::from_str("enabled = true").expect("partial block deserializes");
        assert!(parsed.enabled);
        assert_eq!(parsed.backend_port, DEFAULT_LOCAL_BACKEND_PORT);
        assert!(parsed.apply_local_defaults);
    }

    #[test]
    fn default_port_is_not_a_local_ai_port() {
        // Guards the invariant documented on DEFAULT_LOCAL_BACKEND_PORT: a
        // loopback backend URL on a local-AI port is classified as a model
        // runner by `api::config` and skipped as a backend override.
        let url = format!("http://127.0.0.1:{DEFAULT_LOCAL_BACKEND_PORT}");
        assert!(!crate::api::config::looks_like_local_ai_endpoint(&url));
    }

    #[test]
    fn ephemeral_port_request_is_preserved() {
        let cfg = LocalModeConfig {
            backend_port: 0,
            ..Default::default()
        };
        assert_eq!(cfg.effective_backend_port(), 0);
    }
}
