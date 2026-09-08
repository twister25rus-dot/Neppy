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

    fn server_with(id: &str, model: &str) -> MlxServerConfig {
        MlxServerConfig {
            id: id.to_string(),
            model: model.to_string(),
            ..MlxServerConfig::default()
        }
    }

    #[test]
    fn the_primary_block_takes_the_bare_mlx_slug() {
        // So its provider strings read `mlx:<model>` and match the factory's
        // own MLX branch, which is also what already-saved routing uses.
        assert_eq!(provider_slug_for("primary"), "mlx");
        assert_eq!(provider_slug_for("vision"), "mlx-vision");
    }

    #[test]
    fn starting_a_server_registers_it_as_a_provider() {
        // This is what puts MLX in "Choose provider and model" beside every
        // other provider. Without it a started server is invisible to every
        // model picker in the app.
        let mut config = Config::default();
        let before = config.cloud_providers.len();

        let changed = upsert_provider_entry(
            &mut config,
            &server_with("primary", "mlx-community/Qwen3.8-27B-nvfp4"),
            "http://127.0.0.1:8794/v1",
        );

        assert!(changed);
        assert_eq!(config.cloud_providers.len(), before + 1);
        let entry = config
            .cloud_providers
            .iter()
            .find(|provider| provider.slug == "mlx")
            .expect("an mlx provider entry");
        assert_eq!(entry.label, "MLX");
        assert_eq!(entry.endpoint, "http://127.0.0.1:8794/v1");
        assert_eq!(
            entry.default_model.as_deref(),
            Some("mlx-community/Qwen3.8-27B-nvfp4")
        );
    }

    #[test]
    fn restarting_on_a_new_port_updates_the_endpoint_instead_of_duplicating() {
        // An auto-assigned port moves across restarts; a second entry would
        // leave a stale one pointing at a dead address.
        let mut config = Config::default();
        let server = server_with("primary", "some/model");

        upsert_provider_entry(&mut config, &server, "http://127.0.0.1:8794/v1");
        let changed = upsert_provider_entry(&mut config, &server, "http://127.0.0.1:9001/v1");

        assert!(changed);
        let entries: Vec<_> = config
            .cloud_providers
            .iter()
            .filter(|provider| provider.slug == "mlx")
            .collect();
        assert_eq!(entries.len(), 1, "must not accumulate duplicates");
        assert_eq!(entries[0].endpoint, "http://127.0.0.1:9001/v1");
    }

    #[test]
    fn an_unchanged_server_reports_no_change_so_the_save_can_be_skipped() {
        let mut config = Config::default();
        let server = server_with("primary", "some/model");

        assert!(upsert_provider_entry(
            &mut config,
            &server,
            "http://127.0.0.1:8794/v1"
        ));
        assert!(!upsert_provider_entry(
            &mut config,
            &server,
            "http://127.0.0.1:8794/v1"
        ));
    }

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

// ── provider registration ────────────────────────────────────────────────

/// Provider slug for a server block.
///
/// The block named `primary` takes the bare `mlx` slug, so its provider
/// strings read `mlx:<model>` and match the factory's own MLX branch. Extra
/// blocks are suffixed, and reach the same server through the generic
/// OpenAI-compatible provider path, which is what an MLX server speaks anyway.
pub(crate) fn provider_slug_for(server_id: &str) -> String {
    if server_id == "primary" {
        "mlx".to_string()
    } else {
        let safe: String = server_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        format!("mlx-{safe}")
    }
}

/// Human label for the provider list.
fn provider_label_for(server_id: &str) -> String {
    if server_id == "primary" {
        "MLX".to_string()
    } else {
        format!("MLX ({server_id})")
    }
}

/// Register (or refresh) a running MLX server as a configured provider.
///
/// This is what makes MLX appear in "Choose provider and model" beside every
/// other provider, with its models listed from the server's own `/v1/models`.
/// Before this, a started server was invisible to every model picker in the
/// app and could only be reached by hand-editing routing — which is not what
/// "manage the runtime" should mean.
///
/// Returns whether the config changed, so the caller can skip a save.
pub(crate) fn upsert_provider_entry(
    config: &mut Config,
    server: &crate::neppy::config::schema::MlxServerConfig,
    base_url: &str,
) -> bool {
    use crate::neppy::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let slug = provider_slug_for(&server.id);
    let label = provider_label_for(&server.id);
    // The server takes a bearer token only when the block asked for one.
    let auth_style = if server.uses_bearer() {
        AuthStyle::Bearer
    } else {
        AuthStyle::None
    };
    let default_model = {
        let model = server.model.trim();
        (!model.is_empty()).then(|| model.to_string())
    };

    if let Some(existing) = config
        .cloud_providers
        .iter_mut()
        .find(|provider| provider.slug == slug)
    {
        // A restart usually moves the port, so the endpoint is the field that
        // actually needs refreshing.
        let unchanged = existing.endpoint == base_url
            && existing.label == label
            && existing.auth_style == auth_style
            && existing.default_model == default_model;
        if unchanged {
            return false;
        }
        existing.endpoint = base_url.to_string();
        existing.label = label;
        existing.auth_style = auth_style;
        existing.default_model = default_model;
        return true;
    }

    config.cloud_providers.push(CloudProviderCreds {
        id: format!("mlx-{}", server.id),
        slug,
        label,
        endpoint: base_url.to_string(),
        auth_style,
        default_model,
        ..Default::default()
    });
    true
}
