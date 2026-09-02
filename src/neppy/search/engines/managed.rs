use crate::neppy::config::Config;
use crate::neppy::search::registry::SearchToolParams;
use crate::neppy::tools::Tool;

pub(crate) fn build(root_config: &Config, params: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!(
        requested = %root_config.search.requested_engine_str(),
        "[search] active engine = managed (backend-proxied web_search)"
    );

    vec![Box::new(crate::neppy::search::WebSearchTool::new(
        crate::neppy::integrations::build_client(root_config),
        params.max_results,
        params.timeout_secs,
    ))]
}
