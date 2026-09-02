use std::sync::Arc;

use crate::neppy::config::Config;
use crate::neppy::search::registry::SearchToolParams;
use crate::neppy::tools::Tool;

pub(crate) fn build(root_config: &Config, params: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!("[search] active engine = parallel (BYO direct API)");

    let client = crate::neppy::integrations::build_client(root_config);
    let Some(client) = client else {
        tracing::warn!(
            "[search] engine=parallel but no backend client — falling back to managed surface"
        );
        return vec![Box::new(crate::neppy::search::WebSearchTool::new(
            None,
            params.max_results,
            params.timeout_secs,
        ))];
    };

    vec![
        Box::new(crate::neppy::search::tools::ParallelSearchTool::new(
            Arc::clone(&client),
        )),
        Box::new(crate::neppy::search::tools::ParallelExtractTool::new(
            Arc::clone(&client),
        )),
        Box::new(crate::neppy::search::tools::ParallelChatTool::new(
            Arc::clone(&client),
        )),
        Box::new(crate::neppy::search::tools::ParallelResearchTool::new(
            Arc::clone(&client),
        )),
        Box::new(crate::neppy::search::tools::ParallelEnrichTool::new(
            Arc::clone(&client),
        )),
        Box::new(crate::neppy::search::tools::ParallelDatasetTool::new(
            Arc::clone(&client),
        )),
        Box::new(crate::neppy::search::WebSearchTool::new(
            Some(Arc::clone(&client)),
            params.max_results,
            params.timeout_secs,
        )),
    ]
}
