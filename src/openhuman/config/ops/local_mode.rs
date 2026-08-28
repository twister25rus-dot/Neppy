//! Local Mode config operations.
//!
//! Mirrors [`privacy`](super::privacy): a `get` that reports the current
//! posture plus the service inventory the Settings UI renders, and a `set` that
//! persists the change and republishes it to the enforcement chokepoints so it
//! takes effect without a core restart.
//!
//! # Why the write does not start or stop the server
//!
//! Turning local mode on republishes the state — so hosted round-trips start
//! being refused immediately, which is the safety-critical half — but does not
//! bind the local backend, and turning it off does not unbind it. Binding is
//! boot work: it competes for a port, can fail, and has to happen before any
//! subsystem resolves a backend URL. A half-applied switch (state published,
//! bind failed) would leave the app unable to reach either control plane, so
//! the response asks for a restart instead and says so in its `restart_required`
//! field rather than leaving the UI to guess.

use crate::openhuman::config::Config;
use crate::openhuman::local_mode;
use crate::rpc::RpcOutcome;

use super::loader::load_config_with_timeout;

/// Partial update for the `[local_mode]` block. `None` leaves a field as it is.
#[derive(Debug, Clone, Default)]
pub struct LocalModeSettingsPatch {
    pub enabled: Option<bool>,
    pub backend_port: Option<u16>,
    pub apply_local_defaults: Option<bool>,
    pub proxy_inference: Option<bool>,
}

impl LocalModeSettingsPatch {
    /// Does this patch change anything that only takes effect at boot?
    ///
    /// Only `enabled` and `backend_port` do: the first decides whether the
    /// listener exists, the second where it binds. `apply_local_defaults` and
    /// `proxy_inference` are read per call, so they apply immediately.
    fn needs_restart(&self, config: &Config) -> bool {
        let toggled = self
            .enabled
            .is_some_and(|enabled| enabled != config.local_mode.enabled);
        let repointed = self
            .backend_port
            .is_some_and(|port| port != config.local_mode.backend_port);
        toggled || repointed
    }
}

/// The RPC view of local mode: the settings, the effective state, and the
/// service inventory.
fn local_mode_value(config: &Config, restart_required: bool) -> serde_json::Value {
    let inventory = local_mode::service_inventory();
    serde_json::json!({
        "enabled": config.local_mode.enabled,
        // What the process is *actually* doing, which differs from `enabled`
        // whenever the env override is in play or a restart is still pending.
        "active": local_mode::local_mode_active(),
        "backendPort": config.local_mode.backend_port,
        "backendUrl": local_mode::local_backend_base_url(),
        "applyLocalDefaults": config.local_mode.apply_local_defaults,
        "proxyInference": config.local_mode.proxy_inference,
        "restartRequired": restart_required,
        "services": inventory,
    })
}

/// Read the current local-mode posture and the service inventory.
pub async fn get_local_mode() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    Ok(RpcOutcome::single_log(
        local_mode_value(&config, false),
        "local mode read",
    ))
}

/// Apply a local-mode update to `config`, persist it, and republish the
/// resolved state to the enforcement chokepoints.
pub async fn apply_local_mode_settings(
    config: &mut Config,
    update: LocalModeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let restart_required = update.needs_restart(config);

    if let Some(enabled) = update.enabled {
        log::info!(
            "[local-mode][rpc] enabled: {} -> {}",
            config.local_mode.enabled,
            enabled
        );
        config.local_mode.enabled = enabled;
    }
    if let Some(port) = update.backend_port {
        config.local_mode.backend_port = port;
    }
    if let Some(apply_defaults) = update.apply_local_defaults {
        config.local_mode.apply_local_defaults = apply_defaults;
    }
    if let Some(proxy_inference) = update.proxy_inference {
        config.local_mode.proxy_inference = proxy_inference;
    }

    config.save().await.map_err(|e| e.to_string())?;

    // Republish so the inference factory and the egress spine enforce the new
    // posture immediately. This is the half that must not wait for a restart:
    // leaving it stale would keep hosted round-trips flowing after the user
    // asked for them to stop.
    local_mode::publish_local_mode(local_mode::is_local_mode(config));

    let mut logs = vec![format!(
        "local mode saved to {}",
        config.config_path.display()
    )];
    if restart_required {
        logs.push(
            "restart the core to bind (or release) the local backend on the new setting"
                .to_string(),
        );
    }

    Ok(RpcOutcome::new(
        local_mode_value(config, restart_required),
        logs,
    ))
}

/// Load the configuration, apply the local-mode update, and save it.
pub async fn load_and_apply_local_mode_settings(
    update: LocalModeSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_local_mode_settings(&mut config, update).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(enabled: bool, port: u16) -> Config {
        let mut config = Config::default();
        config.local_mode.enabled = enabled;
        config.local_mode.backend_port = port;
        config
    }

    #[test]
    fn toggling_enabled_needs_a_restart() {
        let config = config_with(false, 43117);
        let patch = LocalModeSettingsPatch {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(patch.needs_restart(&config));
    }

    #[test]
    fn repointing_the_port_needs_a_restart() {
        let config = config_with(true, 43117);
        let patch = LocalModeSettingsPatch {
            backend_port: Some(5000),
            ..Default::default()
        };
        assert!(patch.needs_restart(&config));
    }

    #[test]
    fn writing_the_same_value_needs_no_restart() {
        // A settings panel that PUTs the whole block on every keystroke must
        // not raise a restart banner for a field the user did not change.
        let config = config_with(true, 43117);
        let patch = LocalModeSettingsPatch {
            enabled: Some(true),
            backend_port: Some(43117),
            ..Default::default()
        };
        assert!(!patch.needs_restart(&config));
    }

    #[test]
    fn per_call_settings_apply_without_a_restart() {
        let config = config_with(true, 43117);
        let patch = LocalModeSettingsPatch {
            apply_local_defaults: Some(false),
            proxy_inference: Some(false),
            ..Default::default()
        };
        assert!(
            !patch.needs_restart(&config),
            "both are read per call, so they take effect on the next one"
        );
    }

    #[test]
    fn the_rpc_view_carries_the_service_inventory() {
        let value = local_mode_value(&config_with(true, 43117), false);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["backendPort"], 43117);
        let entries = value["services"]["entries"]
            .as_array()
            .expect("inventory entries");
        assert!(!entries.is_empty());
        assert!(entries.iter().any(|entry| entry["id"] == "auth.session"));
    }

    #[test]
    fn the_rpc_view_reports_active_separately_from_enabled() {
        // `enabled` is the persisted setting; `active` is what the process is
        // doing. They differ across an env override and a pending restart, and
        // a UI that conflated them would tell the user a change had taken
        // effect when it had not.
        let value = local_mode_value(&config_with(true, 43117), true);
        assert!(value.get("active").is_some());
        assert_eq!(value["restartRequired"], true);
    }
}
