//! Tests for local-mode default resolution.
//!
//! The rule under test throughout: **an explicit user choice is never
//! rewritten**. Only defaults nobody picked get redirected.

use super::*;
use crate::openhuman::config::Config;

/// `local_defaults_active` reads `OPENHUMAN_LOCAL_MODE`, a process-global that
/// `resolve_tests` mutates. Serialize on the same crate-wide lock.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::api::config::backend_env_test_lock()
}

fn searxng_configured() -> SearxngConfig {
    SearxngConfig {
        enabled: true,
        base_url: "http://localhost:8080".into(),
        ..Default::default()
    }
}

/// The out-of-the-box block: `enabled = false` with the placeholder
/// `http://localhost:8080` base URL that every install carries whether or not
/// anything listens there.
fn searxng_unconfigured() -> SearxngConfig {
    SearxngConfig::default()
}

// ── local_defaults_active ───────────────────────────────────────────────────

#[test]
fn local_defaults_need_both_switches() {
    let _guard = env_lock();
    let mut config = Config::default();
    assert!(!local_defaults_active(&config), "off by default");

    config.local_mode.enabled = true;
    assert!(local_defaults_active(&config));

    config.local_mode.apply_local_defaults = false;
    assert!(
        !local_defaults_active(&config),
        "opting out of local defaults must keep the topology change but stop \
         rewriting provider choices"
    );
}

// ── embedding_provider ──────────────────────────────────────────────────────

#[test]
fn managed_embeddings_redirect_to_ollama_under_local_mode() {
    for managed in ["managed", "cloud", "openhuman", "MANAGED", "  Cloud  "] {
        assert_eq!(
            embedding_provider(true, managed),
            "ollama",
            "{managed:?} is the backend proxy and cannot serve a local install"
        );
    }
}

#[test]
fn an_unset_embedding_provider_is_treated_as_managed() {
    // Managed is the default by omission; a config that never wrote the key
    // must not keep resolving to the backend proxy.
    assert_eq!(embedding_provider(true, ""), "ollama");
    assert_eq!(embedding_provider(true, "   "), "ollama");
}

#[test]
fn an_explicit_embedding_provider_is_never_rewritten() {
    // `openai` leaves the device, and that is the user's call — local mode
    // drops our backend, not their vendor accounts.
    for chosen in ["openai", "voyage", "cohere", "none", "custom:http://x:1/v1"] {
        assert_eq!(embedding_provider(true, chosen), chosen);
    }
}

#[test]
fn embedding_provider_is_a_no_op_without_local_defaults() {
    assert_eq!(embedding_provider(false, "managed"), "managed");
    assert_eq!(embedding_provider(false, ""), "");
}

#[test]
fn embedding_provider_does_not_allocate_on_the_common_path() {
    // Cow::Borrowed on every branch — the hot path runs per provider build.
    assert!(matches!(
        embedding_provider(false, "managed"),
        std::borrow::Cow::Borrowed(_)
    ));
    assert!(matches!(
        embedding_provider(true, "openai"),
        std::borrow::Cow::Borrowed(_)
    ));
}

// ── search_engine ───────────────────────────────────────────────────────────

#[test]
fn managed_search_becomes_searxng_when_one_is_configured() {
    assert_eq!(
        search_engine(true, SearchEngine::Managed, &searxng_configured()),
        SearchEngine::Searxng
    );
}

#[test]
fn managed_search_becomes_disabled_when_no_searxng_is_configured() {
    // Disabled, not "managed anyway": the agent must know it cannot search, so
    // the tool is absent from its prompt rather than present and always failing.
    assert_eq!(
        search_engine(true, SearchEngine::Managed, &searxng_unconfigured()),
        SearchEngine::Disabled
    );
}

#[test]
fn the_default_placeholder_base_url_does_not_trigger_the_redirect() {
    // `base_url` defaults to http://localhost:8080 on every install, listening
    // or not. Redirecting on it would give every local install a
    // `web_search_tool` that dials a dead port.
    let cfg = SearxngConfig::default();
    assert!(
        !cfg.base_url.trim().is_empty(),
        "precondition: the default is a placeholder URL"
    );
    assert_eq!(
        search_engine(true, SearchEngine::Managed, &cfg),
        SearchEngine::Disabled
    );
}

#[test]
fn an_enabled_instance_with_a_blank_url_is_not_usable() {
    let cfg = SearxngConfig {
        enabled: true,
        base_url: "   ".into(),
        ..Default::default()
    };
    assert_eq!(
        search_engine(true, SearchEngine::Managed, &cfg),
        SearchEngine::Disabled
    );
}

#[test]
fn byok_search_engines_pass_through_untouched() {
    for engine in [
        SearchEngine::Brave,
        SearchEngine::Exa,
        SearchEngine::Querit,
        SearchEngine::Parallel,
    ] {
        assert_eq!(
            search_engine(true, engine, &searxng_configured()),
            engine,
            "{engine:?} calls its vendor directly and is the user's own choice"
        );
    }
}

#[test]
fn an_explicitly_disabled_search_stays_disabled() {
    assert_eq!(
        search_engine(true, SearchEngine::Disabled, &searxng_configured()),
        SearchEngine::Disabled
    );
}

#[test]
fn search_engine_is_a_no_op_without_local_defaults() {
    assert_eq!(
        search_engine(false, SearchEngine::Managed, &searxng_unconfigured()),
        SearchEngine::Managed
    );
}

// ── backend_tracing_enabled ─────────────────────────────────────────────────

#[test]
fn backend_tracing_is_off_under_local_mode_without_a_self_hosted_url() {
    assert!(!backend_tracing_enabled(true, true, None));
    assert!(!backend_tracing_enabled(true, true, Some("  ")));
}

#[test]
fn backend_tracing_survives_when_pointed_at_the_users_own_langfuse() {
    assert!(backend_tracing_enabled(
        true,
        true,
        Some("http://localhost:3000/api/public/ingestion")
    ));
}

#[test]
fn backend_tracing_stays_off_when_the_user_turned_it_off() {
    assert!(!backend_tracing_enabled(
        true,
        false,
        Some("http://localhost:3000")
    ));
    assert!(!backend_tracing_enabled(false, false, None));
}

#[test]
fn backend_tracing_is_a_no_op_without_local_defaults() {
    assert!(backend_tracing_enabled(false, true, None));
}
