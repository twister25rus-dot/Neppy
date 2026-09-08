//! MLX endpoint resolution.
//!
//! Sits beside `lm_studio.rs` and `ollama.rs` and plays the same role: turn a
//! `Config` into the base URL for this runtime.
//!
//! Resolution order, first hit wins:
//!
//! 1. `local_ai.base_url` — an explicit user override, including a server
//!    started outside Neppy
//! 2. `MLX_SERVER_URL` / `OMLX_SERVER_URL` — the env overrides the provider
//!    factory already honours
//! 3. the first `[[mlx.server]]` block with a fixed port
//! 4. the profile default, `http://127.0.0.1:8080/v1`
//!
//! Step 3 only sees blocks with an explicit `port`, because a block with
//! `port = 0` has its port chosen at spawn time and only the running pool
//! knows it. Until the pool lands, a supervised block that wants to be
//! reachable by the inference factory needs a fixed port.

use crate::neppy::config::Config;
use crate::neppy::inference::local::profile::MLX_PROFILE;

/// Base URL for MLX inference, ending in `/v1`.
pub(crate) fn mlx_base_url(config: &Config) -> String {
    if let Some(explicit) = config
        .local_ai
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return explicit.to_string();
    }

    for var in ["MLX_SERVER_URL", "OMLX_SERVER_URL"] {
        if let Some(from_env) = std::env::var(var)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return from_env;
        }
    }

    if let Some(server) = config.mlx.servers.iter().find(|server| server.port != 0) {
        return server.base_url(server.port);
    }

    MLX_PROFILE.default_base_url.to_string()
}

/// Bearer token for MLX requests, when the selected block uses one.
///
/// Prefers the block's own `api_key` over `local_ai.api_key`: the block is
/// what configures the server's `--api-key`, so it is the authoritative half
/// of the pair.
pub(crate) fn mlx_api_key(config: &Config) -> Option<String> {
    if let Some(server) = config
        .mlx
        .servers
        .iter()
        .find(|server| server.uses_bearer() && !server.api_key.trim().is_empty())
    {
        return Some(server.api_key.trim().to_string());
    }

    config
        .local_ai
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::config::schema::MlxServerConfig;

    #[test]
    fn explicit_base_url_wins() {
        let mut config = Config::default();
        config.local_ai.base_url = Some("http://127.0.0.1:9999/v1".to_string());
        assert_eq!(mlx_base_url(&config), "http://127.0.0.1:9999/v1");
    }

    #[test]
    fn a_fixed_port_block_is_used_when_no_override_exists() {
        let mut config = Config::default();
        config.local_ai.base_url = None;
        config.mlx.servers = vec![MlxServerConfig {
            port: 8123,
            ..MlxServerConfig::default()
        }];
        assert_eq!(mlx_base_url(&config), "http://127.0.0.1:8123/v1");
    }

    #[test]
    fn an_auto_port_block_falls_through_to_the_default() {
        // port = 0 is resolved at spawn time; only the pool knows it.
        let mut config = Config::default();
        config.local_ai.base_url = None;
        assert_eq!(config.mlx.servers[0].port, 0);
        assert_eq!(mlx_base_url(&config), MLX_PROFILE.default_base_url);
    }

    #[test]
    fn block_api_key_is_preferred_over_the_local_ai_one() {
        let mut config = Config::default();
        config.local_ai.api_key = Some("from-local-ai".to_string());
        config.mlx.servers = vec![MlxServerConfig {
            auth: "bearer".to_string(),
            api_key: "from-block".to_string(),
            ..MlxServerConfig::default()
        }];
        assert_eq!(mlx_api_key(&config).as_deref(), Some("from-block"));
    }

    #[test]
    fn no_key_when_nothing_configures_one() {
        let config = Config::default();
        assert_eq!(mlx_api_key(&config), None);
    }
}
