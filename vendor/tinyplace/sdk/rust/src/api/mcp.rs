//! Model Context Protocol (MCP) JSON-RPC endpoint. Mirrors
//! `sdk/typescript/src/api/mcp.ts`.

use serde::de::DeserializeOwned;

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::{
    McpInitializeResult, McpJsonRpcRequest, McpJsonRpcResponse, McpRequestOptions, McpResponse,
    McpStreamOptions, McpTerminateResponse,
};

#[derive(Clone)]
pub struct McpApi {
    http: HttpClient,
}

impl McpApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    /// Send a raw JSON-RPC request to the MCP endpoint.
    pub async fn request<R: DeserializeOwned>(
        &self,
        message: &McpJsonRpcRequest,
        options: Option<&McpRequestOptions>,
    ) -> Result<McpResponse<R>> {
        let response = self
            .http
            .post_public_raw("/mcp", Some(message), &mcp_headers(options))
            .await?;
        let session_id = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body: McpJsonRpcResponse<R> = serde_json::from_slice(&response.bytes().await?)?;
        Ok(McpResponse { body, session_id })
    }

    pub async fn initialize(
        &self,
        params: Option<std::collections::HashMap<String, serde_json::Value>>,
        options: Option<&McpRequestOptions>,
    ) -> Result<McpResponse<McpInitializeResult>> {
        let message = McpJsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!(1)),
            method: "initialize".to_string(),
            params,
        };
        self.request(&message, options).await
    }

    pub async fn list_tools(
        &self,
        options: Option<&McpRequestOptions>,
    ) -> Result<McpResponse<serde_json::Value>> {
        let message = McpJsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("tools")),
            method: "tools/list".to_string(),
            params: None,
        };
        self.request(&message, options).await
    }

    pub async fn list_resources(
        &self,
        options: Option<&McpRequestOptions>,
    ) -> Result<McpResponse<serde_json::Value>> {
        let message = McpJsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("resources")),
            method: "resources/list".to_string(),
            params: None,
        };
        self.request(&message, options).await
    }

    pub async fn list_prompts(
        &self,
        options: Option<&McpRequestOptions>,
    ) -> Result<McpResponse<serde_json::Value>> {
        let message = McpJsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(serde_json::json!("prompts")),
            method: "prompts/list".to_string(),
            params: None,
        };
        self.request(&message, options).await
    }

    /// Open the MCP SSE stream (raw HTTP response, caller reads the body).
    pub async fn stream(&self, options: Option<&McpStreamOptions>) -> Result<reqwest::Response> {
        let mut query: Vec<(String, String)> = Vec::new();
        if let Some(resource) = options.and_then(|o| o.resource.as_ref()) {
            query.push(("resource".into(), resource.clone()));
        }
        let headers = stream_headers(options);
        self.http.get_raw("/mcp", &query, &headers).await
    }

    pub async fn terminate(
        &self,
        options: Option<&McpRequestOptions>,
    ) -> Result<McpTerminateResponse> {
        self.http
            .delete_public::<McpTerminateResponse, serde_json::Value>(
                "/mcp",
                None,
                &mcp_headers(options),
            )
            .await
    }
}

fn mcp_headers(options: Option<&McpRequestOptions>) -> Vec<(String, String)> {
    match options.and_then(|o| o.session_id.as_ref()) {
        Some(session_id) => vec![("Mcp-Session-Id".to_string(), session_id.clone())],
        None => Vec::new(),
    }
}

fn stream_headers(options: Option<&McpStreamOptions>) -> Vec<(String, String)> {
    match options.and_then(|o| o.session_id.as_ref()) {
        Some(session_id) => vec![("Mcp-Session-Id".to_string(), session_id.clone())],
        None => Vec::new(),
    }
}
