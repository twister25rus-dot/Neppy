//! Local-mode resolution: is local mode on, and where does the local backend
//! live?
//!
//! Two independent facts, both process-wide:
//!
//! * **Is local mode on** — resolved per call from `config.local_mode.enabled`
//!   with an env override ([`LOCAL_MODE_ENV_VAR`]). Config is the durable
//!   answer; the env var exists for CI, Docker and `openhuman` CLI invocations
//!   that never load a user config.
//! * **Where the local backend is** — a process-global published by the
//!   server once it has bound ([`set_local_backend_base_url`]). It is a
//!   `OnceLock`-style latch rather than a config read because the port may be
//!   ephemeral (`backend_port = 0`), in which case only the bound socket knows
//!   it.
//!
//! [`local_backend_base_url`] falls back to the configured/default port when
//! nothing has been published yet, so a caller that resolves a URL before the
//! server finishes binding gets the right answer for the common (fixed-port)
//! case rather than a panic or an empty string.

use std::sync::RwLock;

use crate::openhuman::config::{
    Config, DEFAULT_LOCAL_BACKEND_PORT, LOCAL_BACKEND_PORT_ENV_VAR, LOCAL_MODE_ENV_VAR,
};

/// Published base URL of the running local backend (e.g.
/// `http://127.0.0.1:43117`). `None` until the server binds.
static LOCAL_BACKEND_BASE_URL: RwLock<Option<String>> = RwLock::new(None);

/// Local mode as published at boot from the loaded config. `None` until the
/// runtime publishes it, in which case [`local_mode_active`] falls back to the
/// environment.
///
/// This mirrors
/// [`live_policy`](crate::openhuman::security::live_policy)'s process-global
/// privacy mode and exists for the same reason: the enforcement chokepoints
/// (the inference factory, the egress spine) are called from deep inside
/// provider construction where no `Config` is in hand, and threading one
/// through every such call site would be a far larger change than the policy
/// it carries.
static LOCAL_MODE_ACTIVE: RwLock<Option<bool>> = RwLock::new(None);

/// Parse the truthy set shared by every OpenHuman boolean env var.
///
/// Truthy: `1`, `true`, `yes`, `on` (case-insensitive, surrounding whitespace
/// ignored). Everything else — including `0`, `false` and an empty value — is
/// `Some(false)`; an unset var is `None` so the caller can distinguish
/// "explicitly off" from "not specified" and let config decide.
fn env_flag(key: &str) -> Option<bool> {
    let raw = std::env::var(key).ok()?;
    let value = raw.trim().to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    Some(matches!(value.as_str(), "1" | "true" | "yes" | "on"))
}

/// Local mode as resolved from the environment alone, for call sites that have
/// no `Config` in hand (early boot, `api::config` URL resolution, CLI entry
/// points).
///
/// Returns `false` when the var is unset — an env-less process is not in local
/// mode unless its config says so.
pub fn is_local_mode_from_env() -> bool {
    env_flag(LOCAL_MODE_ENV_VAR).unwrap_or(false)
}

/// The resolution point every consumer should use.
///
/// Env wins over config **in both directions**: `OPENHUMAN_LOCAL_MODE=0` turns
/// local mode off for a config that enables it, which is what makes the var
/// usable as an escape hatch when a local install needs one hosted round-trip
/// to recover (e.g. re-authenticating against the hosted backend after
/// migrating back).
pub fn is_local_mode(config: &Config) -> bool {
    env_flag(LOCAL_MODE_ENV_VAR).unwrap_or(config.local_mode.enabled)
}

/// The port the local backend should bind, resolved from env then config.
///
/// `0` means "ask the OS for an ephemeral port"; the server publishes the real
/// one through [`set_local_backend_base_url`].
pub fn local_backend_port(config: &Config) -> u16 {
    std::env::var(LOCAL_BACKEND_PORT_ENV_VAR)
        .ok()
        .and_then(|raw| raw.trim().parse::<u16>().ok())
        .unwrap_or_else(|| config.local_mode.effective_backend_port())
}

/// Publish the resolved local-mode state so the enforcement chokepoints can
/// read it without a `Config`. Called once by the runtime after config load,
/// and again whenever the setting changes.
pub fn publish_local_mode(active: bool) {
    tracing::info!(
        active,
        "[local-mode] state published to enforcement chokepoints"
    );
    if let Ok(mut slot) = LOCAL_MODE_ACTIVE.write() {
        *slot = Some(active);
    }
}

/// Clear the published local-mode state (shutdown, and test isolation).
pub fn clear_published_local_mode() {
    if let Ok(mut slot) = LOCAL_MODE_ACTIVE.write() {
        *slot = None;
    }
}

/// Local mode for a call site that has no `Config`.
///
/// Reads the value the runtime published, falling back to
/// [`is_local_mode_from_env`] when nothing has been published — which is the
/// correct answer for the unmanaged contexts (CLI, cron, a bare test binary)
/// where no runtime ever boots, exactly as `current_privacy_mode` defaults to
/// `Standard` there.
pub fn local_mode_active() -> bool {
    if let Ok(slot) = LOCAL_MODE_ACTIVE.read() {
        if let Some(active) = *slot {
            return active;
        }
    }
    is_local_mode_from_env()
}

/// Publish the address the local backend actually bound. Called once by the
/// server; overwrites any previous value so a restart on a new ephemeral port
/// is reflected immediately.
pub fn set_local_backend_base_url(url: impl Into<String>) {
    let url = url.into();
    tracing::info!(url = %url, "[local-mode] local backend address published");
    if let Ok(mut slot) = LOCAL_BACKEND_BASE_URL.write() {
        *slot = Some(url);
    }
}

/// Clear the published address (server shutdown, and test isolation).
pub fn clear_local_backend_base_url() {
    if let Ok(mut slot) = LOCAL_BACKEND_BASE_URL.write() {
        *slot = None;
    }
}

/// Base URL of the local backend.
///
/// Prefers the address the running server published; falls back to the
/// configured/default port so URL resolution works before the listener is up.
/// The fallback deliberately never yields `:0` — an ephemeral request with no
/// published address resolves to [`DEFAULT_LOCAL_BACKEND_PORT`], which is wrong
/// but *addressable*, where `http://127.0.0.1:0` is neither.
pub fn local_backend_base_url() -> String {
    if let Ok(slot) = LOCAL_BACKEND_BASE_URL.read() {
        if let Some(url) = slot.as_deref() {
            return url.to_string();
        }
    }
    format!("http://127.0.0.1:{}", fallback_port())
}

/// Port used by [`local_backend_base_url`] when the server has not published
/// an address: the env override, else [`DEFAULT_LOCAL_BACKEND_PORT`].
///
/// Config is intentionally not consulted here — this runs on paths that have
/// no `Config` (see [`is_local_mode_from_env`]) and a config load at URL
/// resolution time would be a blocking file read on a hot path.
fn fallback_port() -> u16 {
    std::env::var(LOCAL_BACKEND_PORT_ENV_VAR)
        .ok()
        .and_then(|raw| raw.trim().parse::<u16>().ok())
        .filter(|port| *port != 0)
        .unwrap_or(DEFAULT_LOCAL_BACKEND_PORT)
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
