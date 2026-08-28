//! Tests for local-mode resolution.
//!
//! Every case that touches `OPENHUMAN_LOCAL_MODE` / `OPENHUMAN_LOCAL_BACKEND_PORT`
//! takes the crate-wide backend env lock: these are process-globals that
//! `api::config`'s own tests mutate too, and a module-local mutex cannot stop
//! that cross-module race.

use super::*;
use crate::openhuman::config::{
    Config, DEFAULT_LOCAL_BACKEND_PORT, LOCAL_BACKEND_PORT_ENV_VAR, LOCAL_MODE_ENV_VAR,
};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::api::config::backend_env_test_lock()
}

/// RAII guard to snapshot and restore a process-global environment variable.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, val: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds the backend env lock.
        unsafe { std::env::set_var(key, val) };
        Self { key, prev }
    }

    fn remove(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: caller holds the backend env lock.
        unsafe { std::env::remove_var(key) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: caller's lock guard is still alive during drop.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn config_with_local_mode(enabled: bool) -> Config {
    let mut config = Config::default();
    config.local_mode.enabled = enabled;
    config
}

#[test]
fn defaults_to_disabled() {
    let _guard = env_lock();
    let _env = EnvGuard::remove(LOCAL_MODE_ENV_VAR);
    assert!(!is_local_mode(&Config::default()));
    assert!(!is_local_mode_from_env());
}

#[test]
fn config_enables_local_mode_without_env() {
    let _guard = env_lock();
    let _env = EnvGuard::remove(LOCAL_MODE_ENV_VAR);
    assert!(is_local_mode(&config_with_local_mode(true)));
}

#[test]
fn env_enables_local_mode_for_a_config_that_does_not() {
    let _guard = env_lock();
    for truthy in ["1", "true", "TRUE", "yes", "on", "  On  "] {
        let _env = EnvGuard::set(LOCAL_MODE_ENV_VAR, truthy);
        assert!(
            is_local_mode(&config_with_local_mode(false)),
            "{truthy:?} should enable local mode"
        );
        assert!(is_local_mode_from_env(), "{truthy:?} should read as truthy");
    }
}

#[test]
fn env_disables_local_mode_for_a_config_that_enables_it() {
    // The escape hatch: a local install needs exactly one hosted round-trip to
    // migrate back, and editing config.toml to get it would be a worse UX than
    // a single-run env var.
    let _guard = env_lock();
    for falsy in ["0", "false", "no", "off", "nonsense"] {
        let _env = EnvGuard::set(LOCAL_MODE_ENV_VAR, falsy);
        assert!(
            !is_local_mode(&config_with_local_mode(true)),
            "{falsy:?} should disable local mode"
        );
    }
}

#[test]
fn empty_env_value_defers_to_config() {
    // An exported-but-empty var (`OPENHUMAN_LOCAL_MODE=` in a .env) must not
    // read as "explicitly off" — it carries no intent.
    let _guard = env_lock();
    let _env = EnvGuard::set(LOCAL_MODE_ENV_VAR, "   ");
    assert!(is_local_mode(&config_with_local_mode(true)));
    assert!(!is_local_mode(&config_with_local_mode(false)));
}

#[test]
fn backend_port_prefers_env_over_config() {
    let _guard = env_lock();
    let mut config = config_with_local_mode(true);
    config.local_mode.backend_port = 5555;

    let _unset = EnvGuard::remove(LOCAL_BACKEND_PORT_ENV_VAR);
    assert_eq!(local_backend_port(&config), 5555);

    let _env = EnvGuard::set(LOCAL_BACKEND_PORT_ENV_VAR, "6666");
    assert_eq!(local_backend_port(&config), 6666);
}

#[test]
fn backend_port_ignores_an_unparseable_env_value() {
    let _guard = env_lock();
    let mut config = config_with_local_mode(true);
    config.local_mode.backend_port = 5555;
    let _env = EnvGuard::set(LOCAL_BACKEND_PORT_ENV_VAR, "not-a-port");
    assert_eq!(local_backend_port(&config), 5555);
}

#[test]
fn backend_port_zero_requests_an_ephemeral_port() {
    let _guard = env_lock();
    let mut config = config_with_local_mode(true);
    config.local_mode.backend_port = 0;
    let _env = EnvGuard::remove(LOCAL_BACKEND_PORT_ENV_VAR);
    assert_eq!(local_backend_port(&config), 0);
}

#[test]
fn base_url_falls_back_to_the_default_port_before_the_server_binds() {
    let _guard = env_lock();
    let _env = EnvGuard::remove(LOCAL_BACKEND_PORT_ENV_VAR);
    clear_local_backend_base_url();
    assert_eq!(
        local_backend_base_url(),
        format!("http://127.0.0.1:{DEFAULT_LOCAL_BACKEND_PORT}")
    );
}

#[test]
fn base_url_fallback_honours_the_port_env_override() {
    let _guard = env_lock();
    let _env = EnvGuard::set(LOCAL_BACKEND_PORT_ENV_VAR, "7777");
    clear_local_backend_base_url();
    assert_eq!(local_backend_base_url(), "http://127.0.0.1:7777");
}

#[test]
fn base_url_fallback_never_yields_port_zero() {
    // `:0` is a bind-time request, never an address. Falling back to it would
    // hand callers a URL that can only fail to connect.
    let _guard = env_lock();
    let _env = EnvGuard::set(LOCAL_BACKEND_PORT_ENV_VAR, "0");
    clear_local_backend_base_url();
    assert_eq!(
        local_backend_base_url(),
        format!("http://127.0.0.1:{DEFAULT_LOCAL_BACKEND_PORT}")
    );
}

#[test]
fn published_address_wins_over_the_fallback() {
    let _guard = env_lock();
    let _env = EnvGuard::set(LOCAL_BACKEND_PORT_ENV_VAR, "7777");
    set_local_backend_base_url("http://127.0.0.1:49999");
    assert_eq!(local_backend_base_url(), "http://127.0.0.1:49999");
    clear_local_backend_base_url();
}

#[test]
fn publishing_twice_keeps_the_latest_address() {
    // A restart on a fresh ephemeral port must not be shadowed by the address
    // the previous listener published.
    let _guard = env_lock();
    set_local_backend_base_url("http://127.0.0.1:40001");
    set_local_backend_base_url("http://127.0.0.1:40002");
    assert_eq!(local_backend_base_url(), "http://127.0.0.1:40002");
    clear_local_backend_base_url();
}

// ── published local-mode state ──────────────────────────────────────────────

#[test]
fn published_state_wins_over_the_environment() {
    let _guard = env_lock();
    let _env = EnvGuard::remove(LOCAL_MODE_ENV_VAR);
    publish_local_mode(true);
    assert!(local_mode_active());
    publish_local_mode(false);
    assert!(!local_mode_active());
    clear_published_local_mode();
}

#[test]
fn unpublished_state_falls_back_to_the_environment() {
    // CLI, cron and bare test binaries never boot a runtime, so nothing
    // publishes — the env var is the only signal they have.
    let _guard = env_lock();
    clear_published_local_mode();

    let _off = EnvGuard::remove(LOCAL_MODE_ENV_VAR);
    assert!(!local_mode_active());

    let _on = EnvGuard::set(LOCAL_MODE_ENV_VAR, "1");
    assert!(local_mode_active());
}
