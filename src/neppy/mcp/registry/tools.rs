//! LLM-callable wrappers over the `mcp::registry` client surface.
//!
//! These expose the installed-MCP-servers registry to the agent: search the
//! catalog, inspect a server, list installed servers and their connection
//! status, connect/disconnect, call a tool on a connected server, and get
//! AI config help. Thin shims over [`crate::neppy::mcp::registry::ops`].
//!
//! Discovery/observe/connect/call tools are default-ON. The persistent
//! `mcp_registry_install` / `mcp_registry_uninstall` mutators (write installed
//! state + secrets) ship default-OFF via `tools/user_filter.rs`
//! (`mcp_manage` toggle).
//!
//! Permission model (S6): a connected server's tools are third-party code whose
//! effect this host cannot see, so `mcp_registry_tool_call` is classified as an
//! external effect for **every** call — it parks on the approval gate like any
//! other outbound action — and it enforces the security policy's `Act` tier
//! (refused outright on the read-only tier), matching the static-server
//! `mcp_call_tool` bridge. MCP `readOnlyHint` annotations are deliberately not
//! honoured: the registry does not surface them, and even if it did they are
//! self-declared by the remote server, so trusting one would let a hostile
//! server opt its own write tool out of approval. The other mutators
//! (connect / disconnect / install / uninstall) enforce the `Act` tier too, and
//! `install` — which puts new third-party code and its secrets on this machine —
//! is also an external effect.
//!
//! NOTE: the `mcp_setup_*` setup-agent tools and the generic `mcp_list_servers`
//! / `mcp_call_tool` bridge tools already exist elsewhere; these `mcp_registry_*`
//! tools are the distinct installed-registry surface and do not duplicate them.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::neppy::config::Config;
use crate::neppy::security::{SecurityPolicy, ToolOperation};
use crate::neppy::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::ops;

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// Enforce the security policy's `Act` tier for an acting registry tool.
///
/// Reads the live process-global policy first (so an autonomy change made this
/// session applies to the very next call) and falls back to a policy built from
/// this tool's config snapshot — the same live-first discipline the approval gate
/// uses. Refuses on the read-only tier and when the hourly action budget is spent.
fn enforce_act(config: &Config, tool: &str) -> anyhow::Result<()> {
    let policy = crate::neppy::security::live_policy::current().unwrap_or_else(|| {
        Arc::new(SecurityPolicy::from_config(
            &config.autonomy,
            &config.workspace_dir,
            &config.action_dir,
        ))
    });
    policy
        .enforce_tool_operation(ToolOperation::Act, tool)
        .map_err(|reason| {
            tracing::warn!(
                tool,
                "[mcp-registry] acting tool refused by the security policy"
            );
            anyhow::anyhow!(reason)
        })
}

fn req_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// Search the MCP registry catalog.
pub struct McpRegistrySearchTool {
    config: Arc<Config>,
}
impl McpRegistrySearchTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistrySearchTool {
    fn name(&self) -> &str {
        "mcp_registry_search"
    }
    fn description(&self) -> &str {
        "Search the MCP server registry catalog by `query`, optionally filtered by \
         `transport` (\"stdio\" | \"hosted\" | \"all\"), paginated by `page` / \
         `page_size`. Use to discover installable MCP servers."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "transport": { "type": "string", "enum": ["stdio", "hosted", "all"] },
                "page": { "type": "integer", "minimum": 1 },
                "page_size": { "type": "integer", "minimum": 1 }
            }
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::to_string);
        let page = args.get("page").and_then(Value::as_u64).map(|v| v as u32);
        let page_size = args
            .get("page_size")
            .and_then(Value::as_u64)
            .map(|v| v as u32);
        let transport = args
            .get("transport")
            .and_then(Value::as_str)
            .map(str::to_string);
        emit!(
            ops::mcp_clients_registry_search(&self.config, query, transport, page, page_size).await,
            "mcp_registry_search"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Get one registry server by qualified name.
pub struct McpRegistryGetTool {
    config: Arc<Config>,
}
impl McpRegistryGetTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryGetTool {
    fn name(&self) -> &str {
        "mcp_registry_get"
    }
    fn description(&self) -> &str {
        "Get one MCP registry server's detail by `qualified_name`."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "qualified_name": { "type": "string" } },
            "required": ["qualified_name"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        emit!(
            ops::mcp_clients_registry_get(&self.config, qn).await,
            "mcp_registry_get"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List installed MCP servers.
pub struct McpRegistryInstalledListTool {
    config: Arc<Config>,
}
impl McpRegistryInstalledListTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryInstalledListTool {
    fn name(&self) -> &str {
        "mcp_registry_installed_list"
    }
    fn description(&self) -> &str {
        "List the MCP servers currently installed for this user."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            ops::mcp_clients_installed_list(&self.config).await,
            "mcp_registry_installed_list"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Connection status of installed MCP servers.
pub struct McpRegistryStatusTool {
    config: Arc<Config>,
}
impl McpRegistryStatusTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryStatusTool {
    fn name(&self) -> &str {
        "mcp_registry_status"
    }
    fn description(&self) -> &str {
        "Report the connection status of installed MCP servers."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        emit!(
            ops::mcp_clients_status(&self.config).await,
            "mcp_registry_status"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List the tools advertised by a connected MCP server (discovery).
pub struct McpRegistryListToolsTool {
    config: Arc<Config>,
}
impl McpRegistryListToolsTool {
    /// Builds the tool over `config`.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryListToolsTool {
    fn name(&self) -> &str {
        "mcp_registry_list_tools"
    }
    fn description(&self) -> &str {
        "List the tools (name, description, input schema) exposed by a \
         connected MCP server, given its `server_id`. Use this to discover \
         what a connected server can do before calling `mcp_registry_tool_call`. \
         The server must already be connected (see `mcp_registry_status` / \
         `mcp_registry_connect`)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        emit!(
            ops::mcp_clients_list_tools(&self.config, sid).await,
            "mcp_registry_list_tools"
        )
    }
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Connect an installed MCP server.
pub struct McpRegistryConnectTool {
    config: Arc<Config>,
}
impl McpRegistryConnectTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryConnectTool {
    fn name(&self) -> &str {
        "mcp_registry_connect"
    }
    fn description(&self) -> &str {
        "Connect (spawn + handshake) an installed MCP server by `server_id`, \
         returning its tools."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        enforce_act(&self.config, "mcp_registry_connect")?;
        emit!(
            ops::mcp_clients_connect(&self.config, sid).await,
            "mcp_registry_connect"
        )
    }
}

/// Start the browser sign-in for an installed MCP server that needs OAuth.
///
/// The capability-miss route for a server that is installed but cannot be used
/// because it is not signed in (`connect` / `status` report it as
/// unauthorized). Wraps [`ops::mcp_clients_oauth_begin`] — the same discovery +
/// dynamic client registration + PKCE begin step the Connect dialog runs — and
/// hands back the authorize URL for the user to open. The token exchange
/// happens on the app's own `/oauth/mcp/callback` route, so the agent never sees
/// a code or a token; it only has to confirm the result with
/// `mcp_registry_status` / `mcp_registry_connect` afterwards.
///
/// It is an external effect: it contacts the remote server's authorization
/// endpoint and registers this app as an OAuth client there, so it parks on the
/// approval gate like any other outbound action.
pub struct McpRegistryOauthBeginTool {
    config: Arc<Config>,
}
impl McpRegistryOauthBeginTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryOauthBeginTool {
    fn name(&self) -> &str {
        "mcp_registry_oauth_begin"
    }
    fn description(&self) -> &str {
        "Start browser sign-in (OAuth) for an installed MCP server in the `unauthorized` / \
         needs-auth state, by `server_id`. Returns an `authorize_url` for the user to open; \
         after they sign in, confirm with `mcp_registry_status`, then `mcp_registry_connect`."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    /// Contacts a third party's authorization server and registers a client
    /// there, so it asks for approval.
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        enforce_act(&self.config, "mcp_registry_oauth_begin")?;
        tracing::debug!(server_id = %sid, "[mcp-registry] oauth_begin requested by agent");
        let outcome = ops::mcp_clients_oauth_begin(&self.config, sid.clone())
            .await
            .map_err(|e| anyhow::anyhow!("mcp_registry_oauth_begin: {e}"))?;
        let authorize_url = outcome
            .value
            .get("authorize_url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let payload = json!({
            "server_id": sid,
            "status": "awaiting_user_sign_in",
            "authorize_url": authorize_url,
            "next_steps": "Show the authorize_url to the user as a link and ask them to sign in \
                           in their browser. When they say they are done (or after a short wait), \
                           call mcp_registry_status; once the server shows connected, continue \
                           the original task. Never ask the user to paste a code or token.",
        });
        Ok(ToolResult::success(serde_json::to_string(&payload)?))
    }
}

/// Disconnect an MCP server.
pub struct McpRegistryDisconnectTool {
    config: Arc<Config>,
}
impl McpRegistryDisconnectTool {
    /// Builds the tool over `config`.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryDisconnectTool {
    fn name(&self) -> &str {
        "mcp_registry_disconnect"
    }
    fn description(&self) -> &str {
        "Disconnect (stop) a connected MCP server by `server_id`."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        enforce_act(&self.config, "mcp_registry_disconnect")?;
        emit!(
            ops::mcp_clients_disconnect(&self.config, sid).await,
            "mcp_registry_disconnect"
        )
    }
}

/// Call a tool on a connected MCP server.
pub struct McpRegistryToolCallTool {
    config: Arc<Config>,
}
impl McpRegistryToolCallTool {
    /// Builds the tool over `config`.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryToolCallTool {
    fn name(&self) -> &str {
        "mcp_registry_tool_call"
    }
    fn description(&self) -> &str {
        "Invoke a tool on a connected MCP server: `server_id` + `tool_name` + \
         `arguments` object."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "server_id": { "type": "string" },
                "tool_name": { "type": "string" },
                "arguments": { "type": "object" }
            },
            "required": ["server_id", "tool_name"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    /// Every call is an external effect: the remote tool is third-party code
    /// whose effect this host cannot classify (see the module docs on why
    /// `readOnlyHint` is not trusted). The harness parks it on the approval gate.
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        let tool_name = req_str(&args, "tool_name")?;
        let arguments = args.get("arguments").cloned().unwrap_or(json!({}));
        enforce_act(&self.config, "mcp_registry_tool_call")?;
        tracing::debug!(
            server_id = %sid,
            tool = %tool_name,
            "[mcp-registry] tool_call passed the security policy"
        );
        emit!(
            ops::mcp_clients_tool_call(&self.config, sid, tool_name, arguments).await,
            "mcp_registry_tool_call"
        )
    }
}

/// AI config assistance for an MCP server.
pub struct McpRegistryConfigAssistTool {
    config: Arc<Config>,
}
impl McpRegistryConfigAssistTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryConfigAssistTool {
    fn name(&self) -> &str {
        "mcp_registry_config_assist"
    }
    fn description(&self) -> &str {
        "Get AI guidance for configuring an MCP server (`qualified_name`) given a \
         `user_message`; returns a reply and suggested env vars."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": { "type": "string" },
                "user_message": { "type": "string" }
            },
            "required": ["qualified_name", "user_message"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        let msg = req_str(&args, "user_message")?;
        emit!(
            ops::mcp_clients_config_assist(&self.config, qn, msg, None).await,
            "mcp_registry_config_assist"
        )
    }
}

/// Install an MCP server (persists install + env). Default-OFF.
pub struct McpRegistryInstallTool {
    config: Arc<Config>,
}
impl McpRegistryInstallTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryInstallTool {
    fn name(&self) -> &str {
        "mcp_registry_install"
    }
    fn description(&self) -> &str {
        "Install an MCP server (`qualified_name`) with an `env` map and optional \
         `config`. Persists the install + secrets. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "qualified_name": { "type": "string" },
                "env": { "type": "object", "additionalProperties": { "type": "string" } },
                "config": { "type": "object" }
            },
            "required": ["qualified_name"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    /// Installing puts new third-party code and its secrets on this machine, so
    /// it asks for approval like any other outbound action.
    fn external_effect(&self) -> bool {
        true
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let qn = req_str(&args, "qualified_name")?;
        enforce_act(&self.config, "mcp_registry_install")?;
        let env: HashMap<String, String> = args
            .get("env")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| anyhow::anyhow!("mcp_registry_install: invalid env: {e}"))?
            .unwrap_or_default();
        let config_value = args.get("config").cloned();
        emit!(
            ops::mcp_clients_install(&self.config, qn, env, config_value).await,
            "mcp_registry_install"
        )
    }
}

/// Uninstall an MCP server. Default-OFF.
pub struct McpRegistryUninstallTool {
    config: Arc<Config>,
}
impl McpRegistryUninstallTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}
#[async_trait]
impl Tool for McpRegistryUninstallTool {
    fn name(&self) -> &str {
        "mcp_registry_uninstall"
    }
    fn description(&self) -> &str {
        "Uninstall an installed MCP server by `server_id`. Default-OFF (opt-in)."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "server_id": { "type": "string" } },
            "required": ["server_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }
    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let sid = req_str(&args, "server_id")?;
        enforce_act(&self.config, "mcp_registry_uninstall")?;
        emit!(
            ops::mcp_clients_uninstall(&self.config, sid).await,
            "mcp_registry_uninstall"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn names_and_levels() {
        assert_eq!(
            McpRegistrySearchTool::new(cfg()).name(),
            "mcp_registry_search"
        );
        assert_eq!(
            McpRegistrySearchTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            McpRegistryConnectTool::new(cfg()).permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            McpRegistryToolCallTool::new(cfg()).permission_level(),
            PermissionLevel::Execute
        );
        // Discovery tool: read-only, names match.
        assert_eq!(
            McpRegistryListToolsTool::new(cfg()).name(),
            "mcp_registry_list_tools"
        );
        assert_eq!(
            McpRegistryListToolsTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            McpRegistryInstallTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(McpRegistrySearchTool::new(cfg()).scope(), ToolScope::All);
    }

    /// N4: the sign-in starter contacts a third party's authorization server, so
    /// it must reach the approval gate, and it is an acting (Execute) tool.
    #[test]
    fn oauth_begin_is_an_external_effect_and_execute_level() {
        let tool = McpRegistryOauthBeginTool::new(cfg());
        assert_eq!(tool.name(), "mcp_registry_oauth_begin");
        assert!(tool.external_effect());
        assert!(tool.external_effect_with_args(&json!({ "server_id": "s" })));
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
        let required = tool.parameters_schema()["required"].clone();
        assert_eq!(required, json!(["server_id"]));
    }

    #[tokio::test]
    async fn oauth_begin_requires_a_server_id() {
        let tool = McpRegistryOauthBeginTool::new(cfg());
        let err = tool.execute(json!({})).await.unwrap_err().to_string();
        assert!(err.contains("server_id"), "got: {err}");
    }

    /// S6: a connected server's tool is third-party code, so every
    /// `mcp_registry_tool_call` must reach the approval gate, whatever its
    /// arguments — and installing a server must too.
    #[test]
    fn tool_call_and_install_are_external_effects() {
        let call = McpRegistryToolCallTool::new(cfg());
        assert!(call.external_effect());
        assert!(call.external_effect_with_args(&json!({
            "server_id": "s",
            "tool_name": "delete_everything",
            "arguments": {}
        })));
        assert!(call.external_effect_with_args(&json!({
            "server_id": "s",
            "tool_name": "get_weather"
        })));
        assert!(McpRegistryInstallTool::new(cfg()).external_effect());
        // Discovery stays prompt-free.
        assert!(!McpRegistryListToolsTool::new(cfg()).external_effect());
        assert!(!McpRegistryStatusTool::new(cfg()).external_effect());
    }

    fn readonly_tier() -> crate::neppy::security::live_policy::TestPolicyGuard {
        let dir = std::env::temp_dir();
        crate::neppy::security::live_policy::install_scoped(
            Arc::new(SecurityPolicy {
                autonomy: crate::neppy::security::AutonomyLevel::ReadOnly,
                ..SecurityPolicy::default()
            }),
            dir.clone(),
            dir,
        )
    }

    /// S6: on the read-only tier a registry tool call is refused by the security
    /// policy before anything reaches the server (the server here is not even
    /// connected — a refusal that came from the transport would say so).
    #[tokio::test]
    async fn tool_call_is_refused_on_the_readonly_tier() {
        let _tier = readonly_tier();
        let err = McpRegistryToolCallTool::new(cfg())
            .execute(json!({
                "server_id": "definitely-not-connected-uuid",
                "tool_name": "delete_everything",
                "arguments": {}
            }))
            .await
            .expect_err("read-only tier must refuse an MCP tool call");
        let msg = err.to_string();
        assert!(
            msg.contains("read-only"),
            "expected a tier refusal, got: {msg}"
        );
    }

    /// S6 siblings: the other acting registry tools enforce the tier as well.
    #[tokio::test]
    async fn acting_registry_tools_are_refused_on_the_readonly_tier() {
        let _tier = readonly_tier();
        let sid = json!({ "server_id": "definitely-not-connected-uuid" });
        for (name, result) in [
            (
                "connect",
                McpRegistryConnectTool::new(cfg())
                    .execute(sid.clone())
                    .await,
            ),
            (
                "disconnect",
                McpRegistryDisconnectTool::new(cfg())
                    .execute(sid.clone())
                    .await,
            ),
            (
                "uninstall",
                McpRegistryUninstallTool::new(cfg())
                    .execute(sid.clone())
                    .await,
            ),
            (
                "install",
                McpRegistryInstallTool::new(cfg())
                    .execute(json!({ "qualified_name": "@acme/server" }))
                    .await,
            ),
        ] {
            let err = result.expect_err(name);
            assert!(
                err.to_string().contains("read-only"),
                "{name}: expected a tier refusal, got: {err}"
            );
        }
    }

    #[tokio::test]
    async fn get_requires_qualified_name() {
        let err = McpRegistryGetTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing qualified_name");
        assert!(err.to_string().contains("qualified_name"));
    }

    #[tokio::test]
    async fn list_tools_requires_server_id() {
        let err = McpRegistryListToolsTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing server_id");
        assert!(err.to_string().contains("server_id"));
    }

    #[tokio::test]
    async fn list_tools_errors_for_unconnected_server() {
        // A server_id that is not in the live connection map surfaces a
        // "connect first" hint rather than an empty success. Its own
        // workspace keeps the MCP audit store private to this test (the
        // shared default one hit SQLite I/O errors under parallel load).
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = tmp.path().to_path_buf();
        let err = McpRegistryListToolsTool::new(Arc::new(config))
            .execute(json!({ "server_id": "definitely-not-connected-uuid" }))
            .await
            .expect_err("unconnected server must error");
        assert!(
            err.to_string().contains("not connected"),
            "expected connect-first hint, got: {err}"
        );
    }
}
