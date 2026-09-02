//! Tool-schema conversion retained at the Neppy/TinyAgents tool seam.
//!
//! Durable message conversion lives in `agent::message_convert`, beside the
//! Neppy transcript record it adapts. This module remains until WP-4
//! decides the host tool-trait boundary.

use tinyagents::harness::tool::ToolSchema;

use crate::neppy::tools::ToolSpec;

pub(crate) fn spec_to_schema(spec: &ToolSpec) -> ToolSchema {
    ToolSchema::new(
        spec.name.clone(),
        spec.description.clone(),
        spec.parameters.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_tool_schema() {
        let spec = ToolSpec {
            name: "echo".into(),
            description: "echoes".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let schema = spec_to_schema(&spec);
        assert_eq!(schema.name, "echo");
        assert_eq!(schema.parameters, serde_json::json!({"type": "object"}));
    }
}
