//! Per-subsystem default resolution under local mode.
//!
//! Several subsystems default to a *managed* provider — embeddings resolve to
//! the backend's Voyage proxy, `web_search_tool` to the backend's Exa proxy,
//! agent tracing to the backend's Langfuse ingestion. Those defaults are right
//! for a hosted install and wrong for a local one: with no hosted backend they
//! resolve to an endpoint that either refuses the call or is not there at all.
//!
//! Rather than change the defaults themselves (which would alter behaviour for
//! every hosted install), each is re-resolved here when local mode is on. The
//! functions are **pure** — they take the configured value and return the
//! effective one — so the precedence rules are testable without a `Config`, a
//! filesystem, or a network.
//!
//! # Precedence, in one rule
//!
//! **An explicit user choice always wins.** These functions only ever rewrite a
//! value the user did not pick: a `managed` embedding provider is the default
//! nobody chose, so it is redirected; `openai` is a decision, so it is left
//! alone even though it leaves the device. Local mode removes the dependency on
//! *our* backend, not the user's right to call someone else's.

use std::borrow::Cow;

use crate::openhuman::config::{Config, SearchEngine, SearxngConfig};

/// Embedding provider names that mean "the hosted backend's proxy" and are
/// therefore unusable without it.
const MANAGED_EMBEDDING_PROVIDERS: &[&str] = &["managed", "cloud", "openhuman"];

/// The local embedding provider a managed default is redirected to.
const LOCAL_EMBEDDING_PROVIDER: &str = "ollama";

/// Whether local mode should rewrite managed defaults for this config.
///
/// Both switches must be on: `local_mode.enabled` (the topology change) and
/// `local_mode.apply_local_defaults` (the opt-out for users who want to pick
/// every provider by hand).
pub fn local_defaults_active(config: &Config) -> bool {
    super::is_local_mode(config) && config.local_mode.apply_local_defaults
}

/// Resolve the effective embedding provider.
///
/// A managed provider under local mode becomes `ollama`; anything else is the
/// user's own choice and passes through untouched. Returns a [`Cow`] so the
/// overwhelmingly common no-op case does not allocate.
///
/// Note this can still resolve to a provider with no model pulled. That is
/// deliberate: the embeddings factory already degrades to keyword-only memory
/// search when the daemon is unreachable, and silently choosing `none` here
/// would hide a fixable setup problem behind permanently worse recall.
pub fn embedding_provider<'a>(local_defaults: bool, configured: &'a str) -> Cow<'a, str> {
    let trimmed = configured.trim();
    if !local_defaults {
        return Cow::Borrowed(configured);
    }
    // An unset provider is the managed default by omission — treat it the same
    // as naming it, otherwise a config that never wrote the key keeps
    // resolving to the backend proxy.
    let is_managed = trimmed.is_empty()
        || MANAGED_EMBEDDING_PROVIDERS
            .iter()
            .any(|name| trimmed.eq_ignore_ascii_case(name));

    if is_managed {
        Cow::Borrowed(LOCAL_EMBEDDING_PROVIDER)
    } else {
        Cow::Borrowed(configured)
    }
}

/// Resolve the effective search engine.
///
/// [`SearchEngine::Managed`] is the backend's Exa proxy and cannot serve a
/// local install. Under local mode it becomes:
///
/// * [`SearchEngine::Searxng`] when a SearXNG instance is turned on, or
/// * [`SearchEngine::Disabled`] when one is not.
///
/// Disabled rather than "managed anyway" is the load-bearing choice: with no
/// search backend the agent must *know* it cannot search, so the tool is absent
/// from its prompt. Registering a tool that always errors teaches the model to
/// retry a call that can never succeed.
///
/// Every BYOK engine (`brave`, `exa`, `querit`, `parallel`) is an explicit user
/// choice calling that vendor directly, so it passes through — local mode drops
/// *our* backend, not the user's own vendor accounts.
pub fn search_engine(
    local_defaults: bool,
    configured: SearchEngine,
    searxng: &SearxngConfig,
) -> SearchEngine {
    if !local_defaults || configured != SearchEngine::Managed {
        return configured;
    }
    if searxng_is_usable(searxng) {
        SearchEngine::Searxng
    } else {
        SearchEngine::Disabled
    }
}

/// Is there a SearXNG instance worth *automatically* pointing
/// `web_search_tool` at?
///
/// This is a stricter test than the one
/// [`SearchConfig::effective_engine_with_searxng`](crate::openhuman::config::SearchConfig::effective_engine_with_searxng)
/// applies to an explicit `engine = "searxng"`, and deliberately so. `base_url`
/// defaults to `http://localhost:8080` whether or not anything is listening
/// there, so it carries no signal on its own — redirecting on it would hand
/// every local install a `web_search_tool` that dials a dead port. `enabled`
/// is the flag a user sets only once they actually have an instance, so it is
/// the one an *automatic* redirect may key off.
///
/// Explicit selection stays looser because there the user has already said
/// which engine they want; here we are choosing on their behalf.
fn searxng_is_usable(searxng: &SearxngConfig) -> bool {
    searxng.enabled && !searxng.base_url.trim().is_empty()
}

/// Whether backend-proxied agent tracing should run.
///
/// The hosted Langfuse ingestion endpoint lives on the backend, so under local
/// mode it is off unless the user pointed tracing at their own Langfuse — in
/// which case `api_url` names their host and the call never touches ours.
pub fn backend_tracing_enabled(
    local_defaults: bool,
    configured: bool,
    api_url: Option<&str>,
) -> bool {
    if !local_defaults {
        return configured;
    }
    configured && api_url.is_some_and(|url| !url.trim().is_empty())
}

#[cfg(test)]
#[path = "defaults_tests.rs"]
mod tests;
