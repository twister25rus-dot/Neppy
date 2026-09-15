//! Direct mode (BYO API key) ops.

use crate::neppy::config::Config;
use crate::rpc::RpcOutcome;

use super::super::{
    client::{create_direct_composio_tool_for_api_key, direct_list_connections},
    direct_auth,
};
use super::error_utils::OpResult;

/// The reportable tail of a failed key probe.
///
/// `response_error` has already sanitised the upstream body and the API key
/// travels as a header, so nothing secret reaches here — this only trims the
/// internal wrapper prefixes off the front and bounds the length, so the panel
/// shows `HTTP 401: Invalid API key` rather than a nested anyhow chain.
fn probe_failure_detail(rendered: &str) -> String {
    const MAX: usize = 200;
    let collapsed = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
    // Every wrapper in this path prefixes the upstream text, so the last HTTP
    // marker is the upstream answer itself.
    let tail = match collapsed.rfind("HTTP ") {
        Some(at) => collapsed[at..].to_string(),
        None => collapsed,
    };
    if tail.chars().count() <= MAX {
        return tail;
    }
    tail.chars().take(MAX).collect::<String>() + "…"
}

async fn validate_direct_api_key_before_store(config: &Config, api_key: &str) -> OpResult<()> {
    let key_id = direct_auth::fingerprint_api_key(api_key);
    direct_auth::reset_direct_auth_failure(key_id);

    let direct = create_direct_composio_tool_for_api_key(config, api_key)
        .map_err(|e| format!("[composio-direct] validate_api_key: {e}"))?;

    match direct_list_connections(&direct).await {
        Ok(_) => {
            direct_auth::record_direct_auth_success(key_id);
            tracing::debug!("[composio-direct] validate_api_key: probe succeeded");
            Ok(())
        }
        Err(error) => {
            let rendered = format!("{error:#}");
            if direct_auth::is_invalid_api_key_error(&rendered) {
                let detail = probe_failure_detail(&rendered);
                tracing::warn!(
                    detail = %detail,
                    "[composio-direct] validate_api_key: rejected invalid API key before storage"
                );
                // The canonical sentence stays FIRST and verbatim: the
                // observability classifier matches
                // `COMPOSIO_INVALID_API_KEY_ANCHOR` inside it, and the settings
                // panel keys its copy on it. What follows is what Composio
                // actually answered, which is the only part that distinguishes
                // a mistyped key from a revoked one from a key scoped to a
                // different project. Without it every one of those rendered as
                // the same unactionable string, with a key visibly stored and
                // no way to tell which.
                Err(format!(
                    "{} (Composio said: {detail})",
                    direct_auth::COMPOSIO_INVALID_API_KEY_USER_MESSAGE
                ))
            } else {
                tracing::warn!(
                    error = %rendered,
                    "[composio-direct] validate_api_key: probe failed for a non-auth reason; \
                     storing key so the user can retry when Composio is reachable"
                );
                Ok(())
            }
        }
    }
}

/// Read the current Composio routing mode and whether a direct-mode API
/// key is stored. **The key itself is never returned** — only a boolean
/// flag so the UI can show a "Connected" / "Not set" status.
pub async fn composio_get_mode(config: &Config) -> OpResult<RpcOutcome<serde_json::Value>> {
    let mode = config.composio.mode.trim().to_string();
    let key_present = crate::neppy::security::credentials::get_composio_api_key(config)
        .map_err(|e| format!("[composio-direct] get_composio_api_key failed: {e}"))?
        .is_some();
    tracing::debug!(
        mode = %mode,
        key_present = key_present,
        "[composio-direct] get_mode"
    );
    let payload = serde_json::json!({
        "mode": mode,
        "api_key_set": key_present,
    });
    Ok(RpcOutcome::new(
        payload,
        vec![format!(
            "composio: mode={mode}, api_key={}",
            if key_present { "set" } else { "unset" }
        )],
    ))
}

/// Persist a user-provided Composio API key for direct mode and
/// (optionally) flip `config.composio.mode` over to `"direct"`.
///
/// **Logging redacts the key** — only its length and presence are recorded.
pub async fn composio_set_api_key(
    config: &Config,
    api_key: &str,
    activate_direct: bool,
) -> OpResult<RpcOutcome<serde_json::Value>> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio.set_api_key: api_key must not be empty".to_string());
    }
    tracing::debug!(
        key_len = trimmed.len(),
        activate_direct,
        "[composio-direct] set_api_key (redacted)"
    );
    validate_direct_api_key_before_store(config, trimmed).await?;

    crate::neppy::security::credentials::store_composio_api_key(config, trimmed)
        .await
        .map_err(|e| format!("[composio-direct] store_composio_api_key failed: {e}"))?;

    let mode_log = if activate_direct {
        let mut cfg_mut = crate::neppy::config::rpc::load_config_with_timeout()
            .await
            .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
        cfg_mut.composio.mode = crate::neppy::config::schema::COMPOSIO_MODE_DIRECT.into();
        cfg_mut
            .save()
            .await
            .map_err(|e| format!("[composio-direct] save config failed: {e}"))?;
        "mode=direct"
    } else {
        "mode unchanged"
    };

    let effective_mode: String = if activate_direct {
        "direct".to_string()
    } else {
        config.composio.mode.clone()
    };

    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ComposioConfigChanged {
        mode: effective_mode.clone(),
        api_key_set: true,
    });
    tracing::debug!(
        mode = %effective_mode,
        "[composio-cache] published ComposioConfigChanged after set_api_key"
    );

    Ok(RpcOutcome::new(
        serde_json::json!({
            "stored": true,
            "mode": effective_mode,
        }),
        vec![format!("composio: api key stored ({mode_log})")],
    ))
}

/// Clear the stored direct-mode API key and reset
/// `config.composio.mode` back to `"backend"`.
pub async fn composio_clear_api_key(config: &Config) -> OpResult<RpcOutcome<serde_json::Value>> {
    tracing::debug!("[composio-direct] clear_api_key");
    crate::neppy::security::credentials::clear_composio_api_key(config)
        .await
        .map_err(|e| format!("[composio-direct] clear_composio_api_key failed: {e}"))?;

    let mut cfg_mut = crate::neppy::config::rpc::load_config_with_timeout()
        .await
        .map_err(|e| format!("[composio-direct] reload config failed: {e}"))?;
    cfg_mut.composio.mode = crate::neppy::config::schema::COMPOSIO_MODE_BACKEND.into();
    cfg_mut
        .save()
        .await
        .map_err(|e| format!("[composio-direct] save config failed: {e}"))?;
    direct_auth::reset_all_direct_auth_failures();

    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ComposioConfigChanged {
        mode: "backend".to_string(),
        api_key_set: false,
    });
    tracing::debug!("[composio-cache] published ComposioConfigChanged after clear_api_key");

    Ok(RpcOutcome::new(
        serde_json::json!({ "cleared": true, "mode": "backend" }),
        vec!["composio: api key cleared, mode reset to backend".into()],
    ))
}

#[cfg(test)]
mod direct_mode_tests {
    use super::*;

    #[test]
    fn probe_detail_keeps_the_upstream_answer_and_drops_our_wrappers() {
        // Every layer in this path prefixes the upstream text, so the tail from
        // the last HTTP marker is what Composio actually said.
        let rendered = "[composio-direct] validate_api_key: Composio v3 connected_accounts                         failed: HTTP 401: Invalid API key: def**_000 (APIKey_InvalidAPIKey)";

        assert_eq!(
            probe_failure_detail(rendered),
            "HTTP 401: Invalid API key: def**_000 (APIKey_InvalidAPIKey)"
        );
    }

    #[test]
    fn probe_detail_collapses_whitespace_from_a_nested_error_chain() {
        assert_eq!(
            probe_failure_detail("outer:\n   HTTP 500:   upstream   fell over"),
            "HTTP 500: upstream fell over"
        );
    }

    #[test]
    fn probe_detail_falls_back_to_the_whole_message_without_an_http_marker() {
        assert_eq!(
            probe_failure_detail("connection refused"),
            "connection refused"
        );
    }

    #[test]
    fn probe_detail_is_bounded() {
        // Bounded by CHARACTERS, not bytes: slicing a multi-byte error on a
        // byte index would panic on exactly the inputs worth reporting.
        let long = format!("HTTP 500: {}", "é".repeat(400));
        let detail = probe_failure_detail(&long);

        assert_eq!(
            detail.chars().count(),
            201,
            "200 characters plus the ellipsis"
        );
        assert!(detail.ends_with('…'));
    }
}
