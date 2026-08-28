use crate::openhuman::config::{Config, SearchEngine};
use crate::openhuman::tools::Tool;
use std::sync::Arc;

use super::engines;

#[derive(Clone, Copy)]
pub(crate) struct SearchToolParams {
    pub(crate) max_results: usize,
    pub(crate) timeout_secs: u64,
}

/// Build the complete agent-facing search tool surface for the configured
/// search engine.
///
/// Exactly one engine owns the canonical `web_search_tool` slot. When search is
/// disabled, this returns an empty list so search tools are absent from both the
/// agent prompt context and the runtime tool map.
pub fn build_search_tools(root_config: &Config) -> Vec<Box<dyn Tool>> {
    let search = &root_config.search;
    let params = SearchToolParams {
        max_results: search.max_results.clamp(1, 20),
        timeout_secs: search.timeout_secs.max(1),
    };

    // `Config::effective_search_engine` owns the two-step resolution (selection
    // + credential gate, then the local-mode rewrite); `selected` is recomputed
    // here only so the log line below can say what changed and why.
    let selected = search.effective_engine_with_searxng(&root_config.searxng);
    let engine = root_config.effective_search_engine();
    if engine != selected {
        tracing::info!(
            selected = ?selected,
            effective = ?engine,
            "[search] local mode re-resolved the managed search engine"
        );
    }

    let mut tools = match engine {
        SearchEngine::Disabled => engines::disabled::build(root_config, params),
        SearchEngine::Managed => engines::managed::build(root_config, params),
        SearchEngine::Parallel => engines::parallel::build(root_config, params),
        SearchEngine::Brave => engines::brave::build(root_config, params),
        SearchEngine::Querit => engines::querit::build(root_config, params),
        SearchEngine::Exa => engines::exa::build(root_config, params),
        SearchEngine::Searxng => engines::searxng::build(root_config, params),
    };

    // The backend-proxied research tools (TinyFish) ride on the hosted
    // integrations API, so they are skipped in local mode for the same reason
    // the managed engine is — not as a policy choice but because the endpoint
    // they call is not there. `services::entry_for_path` documents the local
    // alternative (browser + fetch tools, or an MCP server with the user's own
    // key).
    if engine != SearchEngine::Disabled && !crate::openhuman::local_mode::is_local_mode(root_config)
    {
        tools.extend(build_backend_search_tools(root_config));
    }

    tools
}

fn build_backend_search_tools(root_config: &Config) -> Vec<Box<dyn Tool>> {
    let Some(client) = crate::openhuman::integrations::build_client(root_config) else {
        tracing::debug!("[search] no integration client — backend search tools skipped");
        return Vec::new();
    };

    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    if root_config.integrations.tinyfish.is_active() {
        tools.push(Box::new(
            crate::openhuman::search::tools::TinyFishSearchTool::new(Arc::clone(&client)),
        ));
        tools.push(Box::new(
            crate::openhuman::search::tools::TinyFishFetchTool::new(Arc::clone(&client)),
        ));
        tools.push(Box::new(
            crate::openhuman::search::tools::TinyFishAgentRunTool::new(Arc::clone(&client)),
        ));
        tracing::debug!("[search] registered tinyfish tools");
    } else {
        tracing::debug!("[search] tinyfish disabled — skipping");
    }

    tools
}

#[cfg(test)]
mod tests {
    use crate::openhuman::config::Config;

    /// Serialize the local-mode cases on the crate-wide backend env lock.
    ///
    /// `local_defaults_active` reads `OPENHUMAN_LOCAL_MODE`, which
    /// `local_mode::resolve`'s tests set and clear. Without the lock a
    /// concurrent `OPENHUMAN_LOCAL_MODE=0` would make these fail
    /// intermittently — the kind of flake that gets re-run rather than fixed.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::api::config::backend_env_test_lock()
    }

    #[test]
    fn disabled_engine_registers_no_search_tools() {
        let mut cfg = Config::default();
        cfg.search.engine = "disabled".to_string();

        let tools = super::build_search_tools(&cfg);

        assert!(tools.is_empty());
    }

    #[test]
    fn managed_engine_registers_unified_web_search_tool() {
        let mut cfg = Config::default();
        cfg.search.engine = "managed".to_string();

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(names, vec!["web_search_tool"]);
    }

    #[test]
    fn exa_engine_registers_the_byok_exa_family() {
        let mut cfg = Config::default();
        cfg.search.engine = "exa".to_string();
        cfg.search.exa.api_key = Some("test-key".to_string());

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "web_search_tool",
                "exa_search",
                "exa_find_similar",
                "exa_get_contents"
            ]
        );
    }

    #[test]
    fn exa_without_a_key_falls_back_to_the_managed_surface() {
        let mut cfg = Config::default();
        cfg.search.engine = "exa".to_string();

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(names, vec!["web_search_tool"]);
    }

    #[test]
    fn searxng_engine_registers_the_canonical_slot_and_its_own_tool() {
        // `web_search_tool` is the name every system prompt and skill knows.
        // An engine that only registered under its own name would be invisible
        // to all of them.
        let mut cfg = Config::default();
        cfg.search.engine = "searxng".to_string();
        cfg.searxng.base_url = "http://localhost:8080".to_string();

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(names, vec!["web_search_tool", "searxng_search"]);
    }

    #[test]
    fn local_mode_redirects_the_managed_engine_to_a_configured_searxng() {
        let _guard = env_lock();
        let mut cfg = Config::default();
        cfg.local_mode.enabled = true;
        cfg.searxng.enabled = true;
        cfg.searxng.base_url = "http://localhost:8080".to_string();
        // `managed` is the default nobody chose, and it is the backend's Exa
        // proxy — unusable with no backend.
        assert_eq!(cfg.search.requested_engine_str(), "managed");

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
        assert_eq!(names, vec!["web_search_tool", "searxng_search"]);
    }

    #[test]
    fn local_mode_without_a_searxng_registers_no_search_tools() {
        let _guard = env_lock();
        // Disabled rather than "managed anyway": the agent must know it cannot
        // search, so the tool is absent from its prompt. A tool that is present
        // and always fails teaches the model to retry a doomed call.
        let mut cfg = Config::default();
        cfg.local_mode.enabled = true;

        assert!(super::build_search_tools(&cfg).is_empty());
    }

    #[test]
    fn local_mode_leaves_a_byo_engine_alone() {
        let _guard = env_lock();
        // Local mode drops *our* backend, not the user's own vendor account.
        let mut cfg = Config::default();
        cfg.local_mode.enabled = true;
        cfg.search.engine = "brave".to_string();
        cfg.search.brave.api_key = Some("real".to_string());

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
        assert!(names.contains(&"web_search_tool"));
        assert!(names.contains(&"brave_news_search"));
    }

    #[test]
    fn brave_engine_registers_brave_search_family() {
        let mut cfg = Config::default();
        cfg.search.engine = "brave".to_string();
        cfg.search.brave.api_key = Some("test-key".to_string());

        let tools = super::build_search_tools(&cfg);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "web_search_tool",
                "brave_news_search",
                "brave_image_search",
                "brave_video_search"
            ]
        );
    }
}
