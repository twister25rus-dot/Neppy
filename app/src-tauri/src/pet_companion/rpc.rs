//! Core RPC seam for the Pet companion shell (§2.9).
//!
//! `CompanionRpc` is the only way the shell talks to the core. Production
//! posts JSON-RPC to the embedded core with the per-launch bearer
//! (`core_rpc::apply_auth`, same as `imessage_scanner`); tests use a fake.
//! Nothing here handles content: only lease/pause/resume/ask/capture control
//! calls and small status fields.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const METHOD_LEASE: &str = "openhuman.pet_companion_lease";
pub(crate) const METHOD_PAUSE: &str = "openhuman.pet_companion_pause";
pub(crate) const METHOD_RESUME: &str = "openhuman.pet_companion_resume";
pub(crate) const METHOD_ASK: &str = "openhuman.pet_companion_ask";
pub(crate) const METHOD_CAPTURE: &str = "openhuman.pet_companion_capture";

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// `capture` blocks until the user finishes selecting (core caps it at 60 s).
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(70);

/// Where a pause/resume/ask/capture request originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Hotkey,
    Tray,
}

impl Source {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Hotkey => "hotkey",
            Self::Tray => "tray",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct HotkeyError {
    pub(crate) name: String,
    pub(crate) error: String,
}

/// `companion_lease` params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LeaseRequest {
    pub(crate) visible: bool,
    pub(crate) hotkey_errors: Vec<HotkeyError>,
}

impl LeaseRequest {
    pub(crate) fn to_params(&self) -> Value {
        let mut p = json!({ "indicator": "tray", "visible": self.visible });
        if !self.hotkey_errors.is_empty() {
            p["hotkey_errors"] = serde_json::to_value(&self.hotkey_errors).unwrap_or(Value::Null);
        }
        p
    }
}

/// Default accelerators from D6, used when the core omits a field.
pub(crate) fn default_accelerator(action: &str, macos: bool) -> String {
    let mods = if macos {
        "Alt+Shift+Cmd"
    } else {
        "Alt+Shift+Ctrl"
    };
    let key = match action {
        "pause" => "P",
        "ask" => "Space",
        _ => "S",
    };
    format!("{mods}+{key}")
}

fn d_pause() -> Option<String> {
    Some(default_accelerator("pause", cfg!(target_os = "macos")))
}
fn d_ask() -> Option<String> {
    Some(default_accelerator("ask", cfg!(target_os = "macos")))
}
fn d_capture() -> Option<String> {
    Some(default_accelerator("capture", cfg!(target_os = "macos")))
}

/// Effective accelerators the core wants registered. A missing field means
/// "default"; `null` or an empty string means "disabled".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct HotkeyConfig {
    #[serde(default = "d_pause")]
    pub(crate) pause: Option<String>,
    #[serde(default = "d_ask")]
    pub(crate) ask: Option<String>,
    #[serde(default = "d_capture")]
    pub(crate) capture: Option<String>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            pause: d_pause(),
            ask: d_ask(),
            capture: d_capture(),
        }
    }
}

/// `companion_lease` result. Every field tolerates absence.
///
/// `screen_capture_active` is a contract addition for T2c: true while the
/// autonomous screen-capture loop is armed (D2), so the tray can say
/// "observing screen".
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct LeaseResponse {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) paused: bool,
    #[serde(default)]
    pub(crate) platform_supported: bool,
    #[serde(default)]
    pub(crate) screen_capture_active: bool,
    #[serde(default)]
    pub(crate) hotkeys: HotkeyConfig,
}

/// The few `CompanionStatus` fields the shell reads from pause/resume.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub(crate) struct StatusLite {
    #[serde(default)]
    pub(crate) state: String,
    #[serde(default)]
    pub(crate) paused: bool,
    #[serde(default)]
    pub(crate) screen_capture_active: bool,
}

/// `companion_ask` / `companion_capture` result; the suggestion itself
/// reaches the UI over the socket, not through the shell.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub(crate) struct ActionOutcome {
    #[serde(default)]
    pub(crate) status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RpcError {
    /// Token not initialised yet (core still booting).
    NotReady(String),
    Transport(String),
    Remote(String),
    Parse(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady(m) => write!(f, "core not ready: {m}"),
            Self::Transport(m) => write!(f, "transport: {m}"),
            Self::Remote(m) => write!(f, "remote: {m}"),
            Self::Parse(m) => write!(f, "parse: {m}"),
        }
    }
}

#[async_trait]
pub(crate) trait CompanionRpc: Send + Sync {
    async fn lease(&self, req: &LeaseRequest) -> Result<LeaseResponse, RpcError>;
    async fn pause(&self, source: Source) -> Result<StatusLite, RpcError>;
    async fn resume(&self, source: Source) -> Result<StatusLite, RpcError>;
    async fn ask(&self, source: Source) -> Result<ActionOutcome, RpcError>;
    async fn capture(&self, source: Source) -> Result<ActionOutcome, RpcError>;
}

/// Strip the JSON-RPC envelope and the `{result, logs}` outcome envelope, and
/// surface a JSON-RPC `error` as `RpcError::Remote`.
pub(crate) fn unwrap_rpc_value(mut v: Value) -> Result<Value, RpcError> {
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| err.to_string());
        return Err(RpcError::Remote(msg));
    }
    if v.get("jsonrpc").is_some() {
        v = v.get_mut("result").map(Value::take).unwrap_or(Value::Null);
    }
    let is_outcome = v
        .as_object()
        .map(|o| o.contains_key("result") && o.keys().all(|k| k == "result" || k == "logs"))
        .unwrap_or(false);
    if is_outcome {
        v = v.get_mut("result").map(Value::take).unwrap_or(Value::Null);
    }
    Ok(v)
}

fn parse_as<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, RpcError> {
    let inner = unwrap_rpc_value(v)?;
    serde_json::from_value(inner).map_err(|e| RpcError::Parse(e.to_string()))
}

/// Tolerant lease-response parser (accepts raw, `{result,logs}` and full
/// JSON-RPC envelopes).
pub(crate) fn parse_lease_response(v: Value) -> Result<LeaseResponse, RpcError> {
    parse_as(v)
}

pub(crate) fn parse_status(v: Value) -> Result<StatusLite, RpcError> {
    parse_as(v)
}

pub(crate) fn parse_outcome(v: Value) -> Result<ActionOutcome, RpcError> {
    parse_as(v)
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Production implementation: authenticated POST to the in-process core.
pub(crate) struct HttpCompanionRpc;

impl HttpCompanionRpc {
    async fn call(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RpcError> {
        let url = crate::core_rpc::core_rpc_url_value();
        let req = crate::core_rpc::apply_auth(client().post(&url)).map_err(RpcError::NotReady)?;
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        log::trace!("[pet::companion::shell] rpc -> {method}");
        let res = req
            .timeout(timeout)
            .json(&body)
            .send()
            .await
            .map_err(|e| RpcError::Transport(e.without_url().to_string()))?;
        let status = res.status();
        let value: Value = res
            .json()
            .await
            .map_err(|e| RpcError::Parse(format!("http {status}: {}", e.without_url())))?;
        if !status.is_success() && value.get("error").is_none() {
            return Err(RpcError::Transport(format!("http {status}")));
        }
        Ok(value)
    }
}

#[async_trait]
impl CompanionRpc for HttpCompanionRpc {
    async fn lease(&self, req: &LeaseRequest) -> Result<LeaseResponse, RpcError> {
        parse_lease_response(
            self.call(METHOD_LEASE, req.to_params(), CALL_TIMEOUT)
                .await?,
        )
    }
    async fn pause(&self, source: Source) -> Result<StatusLite, RpcError> {
        let p = json!({ "source": source.as_str() });
        parse_status(self.call(METHOD_PAUSE, p, CALL_TIMEOUT).await?)
    }
    async fn resume(&self, source: Source) -> Result<StatusLite, RpcError> {
        let p = json!({ "source": source.as_str() });
        parse_status(self.call(METHOD_RESUME, p, CALL_TIMEOUT).await?)
    }
    async fn ask(&self, source: Source) -> Result<ActionOutcome, RpcError> {
        let p = json!({ "source": source.as_str() });
        parse_outcome(self.call(METHOD_ASK, p, CALL_TIMEOUT).await?)
    }
    async fn capture(&self, source: Source) -> Result<ActionOutcome, RpcError> {
        let p = json!({ "source": source.as_str() });
        parse_outcome(self.call(METHOD_CAPTURE, p, CAPTURE_TIMEOUT).await?)
    }
}
