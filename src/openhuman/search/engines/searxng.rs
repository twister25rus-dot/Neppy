use crate::openhuman::config::Config;
use crate::openhuman::search::registry::SearchToolParams;
use crate::openhuman::tools::Tool;

/// Register SearXNG as the canonical search engine.
///
/// SearXNG is a metasearch front-end the user runs themselves, so unlike every
/// other engine here it needs no third-party account and no key — which is why
/// Local Mode resolves the managed default to it (see
/// [`local_mode::defaults::search_engine`](crate::openhuman::local_mode::defaults::search_engine)).
///
/// Two tools are registered from one implementation:
///
/// * `web_search_tool` — the canonical slot every system prompt and skill
///   names. Without this the agent cannot search at all, because nothing in the
///   prompt knows the word "searxng".
/// * `searxng_search` — the engine-specific surface, which exposes SearXNG's
///   `categories` parameter. Mirrors how the `brave` and `querit` engines
///   register their own richer tools next to the canonical one.
///
/// Result limits come from `[searxng]`, not from the unified `[search]` block:
/// `[search] max_results` is capped at 20 while SearXNG accepts up to 50, and
/// silently narrowing a self-hosted instance to the unified cap would lose
/// results the user's own server was willing to return.
pub(crate) fn build(root_config: &Config, params: SearchToolParams) -> Vec<Box<dyn Tool>> {
    let searxng = &root_config.searxng;
    tracing::debug!(
        base_url = %searxng.base_url,
        "[search] active engine = searxng (self-hosted, keyless)"
    );

    // `[search] timeout_secs` is the unified request budget and applies to
    // whichever engine fills the slot, so the canonical tool honours it; the
    // engine-specific tool keeps SearXNG's own timeout for parity with how it
    // behaves when registered alongside a different engine.
    vec![
        Box::new(
            crate::openhuman::search::tools::SearxngSearchTool::new_web_search_tool(
                searxng.base_url.clone(),
                searxng.max_results,
                searxng.default_language.clone(),
                params.timeout_secs,
            ),
        ),
        Box::new(crate::openhuman::search::tools::SearxngSearchTool::new(
            searxng.base_url.clone(),
            searxng.max_results,
            searxng.default_language.clone(),
            searxng.timeout_secs,
        )),
    ]
}
