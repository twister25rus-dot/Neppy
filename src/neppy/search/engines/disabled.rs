use crate::neppy::config::Config;
use crate::neppy::search::registry::SearchToolParams;
use crate::neppy::tools::Tool;

pub(crate) fn build(_: &Config, _: SearchToolParams) -> Vec<Box<dyn Tool>> {
    tracing::debug!("[search] disabled — no search tools registered");
    Vec::new()
}
