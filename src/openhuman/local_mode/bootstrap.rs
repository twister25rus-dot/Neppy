//! Bringing local mode up at core boot.
//!
//! Called from [`CoreContext::init_with_config`](crate::core::runtime::CoreContext)
//! immediately after the config resolves and **before** any subsystem starts,
//! because the two things it does are preconditions for everything after it:
//!
//! 1. [`publish_local_mode`](super::publish_local_mode) — the enforcement
//!    chokepoints (the inference factory, the egress spine) read the published
//!    value, and a subsystem that boots before it is published would resolve
//!    against the wrong topology.
//! 2. Start the local backend — [`effective_backend_api_url`](crate::api::config::effective_backend_api_url)
//!    resolves to it, so anything that makes a backend call before the listener
//!    binds gets a connection refused.
//!
//! # Idempotence
//!
//! `init_with_config` runs once per process in production but more than once in
//! tests and in embedders that rebuild a runtime. Starting twice would fail on
//! the second bind (or, with an ephemeral port, silently strand the first
//! listener and publish the second). [`start_local_backend_once`] therefore
//! returns early when a server is already running.

use crate::openhuman::config::Config;

#[cfg(feature = "http-server")]
use std::sync::Mutex;

#[cfg(feature = "http-server")]
use super::backend::LocalBackendHandle;

/// The running local backend, held for the process lifetime.
///
/// A `Mutex<Option<_>>` rather than a `OnceLock` because the server can be
/// stopped and restarted (a config change that flips local mode), which a
/// write-once cell cannot express.
#[cfg(feature = "http-server")]
static RUNNING: Mutex<Option<LocalBackendHandle>> = Mutex::new(None);

/// Publish local mode and, when it is on, start the local backend.
///
/// `config` is `None` when config resolution failed at boot; local mode then
/// falls back to the environment. That is the right degradation: a config the
/// core could not read must not silently drop a user into hosted mode and start
/// making backend calls they had turned off.
pub async fn bootstrap(config: Option<&Config>) {
    let active = match config {
        Some(config) => super::is_local_mode(config),
        None => {
            let from_env = super::is_local_mode_from_env();
            tracing::warn!(
                local_mode = from_env,
                "[local-mode] config unavailable at boot — resolving local mode from the \
                 environment alone"
            );
            from_env
        }
    };

    super::publish_local_mode(active);

    if !active {
        return;
    }

    #[cfg(feature = "http-server")]
    {
        // A caller-supplied config is authoritative; without one, the port
        // resolution below falls back to env/default, which is the same answer
        // `local_backend_base_url` gives, so the two agree.
        let owned;
        let config = match config {
            Some(config) => config,
            None => {
                owned = Config::default();
                &owned
            }
        };
        start_local_backend_once(config).await;
    }

    #[cfg(not(feature = "http-server"))]
    {
        let _ = config;
        // A slim build has no axum listener to bind. Local mode's *policy*
        // still applies (the enforcement chokepoints read the published value
        // and refuse hosted round-trips), but nothing serves the control plane,
        // so say so rather than leaving the operator to infer it from a
        // connection-refused later.
        tracing::warn!(
            "[local-mode] enabled, but this build has no HTTP transport \
             (`http-server` feature off) — hosted round-trips are refused and nothing \
             serves the local control plane"
        );
    }
}

/// Start the local backend unless one is already running.
#[cfg(feature = "http-server")]
async fn start_local_backend_once(config: &Config) {
    {
        let running = match RUNNING.lock() {
            Ok(running) => running,
            Err(poisoned) => {
                // A panic in a previous holder leaves the lock poisoned. The
                // guarded value is a handle, not an invariant that a panic
                // could have corrupted, so recovering is safe and strictly
                // better than taking the boot down.
                tracing::warn!("[local-mode] handle lock was poisoned — recovering");
                poisoned.into_inner()
            }
        };
        if let Some(handle) = running.as_ref() {
            tracing::debug!(
                addr = %handle.addr(),
                "[local-mode] local backend already running — not starting a second"
            );
            return;
        }
    }

    match super::backend::start(config).await {
        Ok(handle) => {
            tracing::info!(
                base_url = %handle.base_url(),
                "[local-mode] local backend started — hosted control plane is served locally"
            );
            {
                let mut running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
                *running = Some(handle);
            }
            register_shutdown_hook();
        }
        Err(error) => {
            // Do not take the boot down. Local mode stays published, so every
            // hosted round-trip is refused with an explained error rather than
            // silently escaping to `api.tinyhumans.ai` — which is the failure
            // this whole module exists to prevent. The app is degraded but
            // still honest, and the operator gets an actionable message.
            tracing::error!(
                error = %error,
                "[local-mode] local backend FAILED to start — control-plane calls will be \
                 refused until it can bind"
            );
        }
    }
}

/// Register the process shutdown hook that stops the listener, once.
///
/// Registered on first successful start rather than unconditionally at boot,
/// so a build that never turns local mode on does not carry a no-op hook. The
/// `Once` matters because a stop/start cycle would otherwise queue a second
/// hook that runs `shutdown_local_backend` against an already-empty slot.
#[cfg(feature = "http-server")]
fn register_shutdown_hook() {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        crate::core::shutdown::register(|| async {
            shutdown_local_backend().await;
        });
    });
}

/// Stop the local backend if one is running. Used on shutdown and when local
/// mode is turned off at runtime.
#[cfg(feature = "http-server")]
pub async fn shutdown_local_backend() {
    let handle = {
        let mut running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
        running.take()
    };
    if let Some(handle) = handle {
        handle.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::api::config::backend_env_test_lock()
    }

    #[tokio::test]
    async fn bootstrap_publishes_disabled_for_a_default_config() {
        let _guard = env_lock();
        // SAFETY: the backend env lock is held.
        unsafe { std::env::remove_var(crate::openhuman::config::LOCAL_MODE_ENV_VAR) };

        bootstrap(Some(&Config::default())).await;
        assert!(!super::super::local_mode_active());
        super::super::resolve::clear_published_local_mode();
    }

    #[tokio::test]
    async fn bootstrap_without_a_config_falls_back_to_the_environment() {
        // A config the core could not read must not silently drop the user
        // into hosted mode.
        let _guard = env_lock();
        // SAFETY: the backend env lock is held.
        unsafe { std::env::set_var(crate::openhuman::config::LOCAL_MODE_ENV_VAR, "1") };

        bootstrap(None).await;
        assert!(super::super::local_mode_active());

        // SAFETY: the backend env lock is held.
        unsafe { std::env::remove_var(crate::openhuman::config::LOCAL_MODE_ENV_VAR) };
        super::super::resolve::clear_published_local_mode();
        #[cfg(feature = "http-server")]
        shutdown_local_backend().await;
    }

    #[cfg(feature = "http-server")]
    #[tokio::test]
    async fn starting_twice_keeps_the_first_listener() {
        let _guard = env_lock();
        let mut config = Config::default();
        config.local_mode.enabled = true;
        config.local_mode.backend_port = 0;

        shutdown_local_backend().await;
        start_local_backend_once(&config).await;
        let first = super::super::local_backend_base_url();

        start_local_backend_once(&config).await;
        assert_eq!(
            super::super::local_backend_base_url(),
            first,
            "a second start must not strand the first listener and publish a new address"
        );

        shutdown_local_backend().await;
        super::super::resolve::clear_published_local_mode();
    }
}
