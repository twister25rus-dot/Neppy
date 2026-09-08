//! RPC surface for the managed MLX runtime — namespace `mlx`.
//!
//! Handlers stay thin: the supervisor in
//! `service::mlx_admin::pool` already owns the logic, so these translate
//! params, call it, and shape the reply. Everything routes through the one
//! `LocalAiService` singleton, because the pool holds live child processes and
//! a second instance would supervise nothing.
//!
//! Registered under `DomainGroup::Inference` in `core::all`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::neppy::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

/// Log lines returned when a caller does not ask for a specific count.
const DEFAULT_LOG_LIMIT: usize = 100;
const MAX_LOG_LIMIT: usize = 1000;

#[derive(Debug, Deserialize)]
struct ServerIdParams {
    id: String,
}

#[derive(Debug, Deserialize)]
struct LogsParams {
    id: String,
    #[serde(default)]
    limit: Option<usize>,
}

/// Reply for `mlx.status`: every configured block plus the shared budget.
#[derive(Debug, Serialize)]
struct MlxStatusReply {
    enabled: bool,
    embeddings_backend: String,
    /// Resident memory across managed servers, GiB.
    memory_used_gib: f64,
    /// Ceiling those servers share, GiB.
    memory_budget_gib: f64,
    /// Configuration problems worth showing before a start is attempted.
    problems: Vec<String>,
    servers: Vec<super::service::mlx_admin::pool::MlxServerStatus>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("status"),
        schemas("start"),
        schemas("stop"),
        schemas("restart"),
        schemas("logs"),
        schemas("unload"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("start"),
            handler: handle_start,
        },
        RegisteredController {
            schema: schemas("stop"),
            handler: handle_stop,
        },
        RegisteredController {
            schema: schemas("restart"),
            handler: handle_restart,
        },
        RegisteredController {
            schema: schemas("logs"),
            handler: handle_logs,
        },
        RegisteredController {
            schema: schemas("unload"),
            handler: handle_unload,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "mlx",
            function: "status",
            description: "State of every configured MLX server, with memory use against the \
                          shared budget.",
            inputs: vec![],
            outputs: vec![json_output(
                "status",
                "Servers, memory summary and config problems.",
            )],
        },
        "start" => ControllerSchema {
            namespace: "mlx",
            function: "start",
            description: "Start one MLX server. Refuses when the model would not fit in the \
                          remaining memory budget.",
            inputs: vec![required_string(
                "id",
                "The [[mlx.server]] block id to start.",
            )],
            outputs: vec![json_output("server", "Status of the started server.")],
        },
        "stop" => ControllerSchema {
            namespace: "mlx",
            function: "stop",
            description: "Stop one MLX server. Stopping one that is not running is not an error.",
            inputs: vec![required_string(
                "id",
                "The [[mlx.server]] block id to stop.",
            )],
            outputs: vec![json_output("stopped", "Confirmation payload.")],
        },
        "restart" => ControllerSchema {
            namespace: "mlx",
            function: "restart",
            description: "Restart one MLX server, picking up any edited parameters.",
            inputs: vec![required_string(
                "id",
                "The [[mlx.server]] block id to restart.",
            )],
            outputs: vec![json_output("server", "Status of the restarted server.")],
        },
        "logs" => ControllerSchema {
            namespace: "mlx",
            function: "logs",
            description: "Recent stdout/stderr from one MLX server.",
            inputs: vec![
                required_string("id", "The [[mlx.server]] block id."),
                optional_u64(
                    "limit",
                    "Maximum lines to return. Defaults to 100, capped at 1000.",
                ),
            ],
            outputs: vec![json_output("logs", "Log lines, oldest first.")],
        },
        "unload" => ControllerSchema {
            namespace: "mlx",
            function: "unload",
            description: "Ask a running MLX server to release its loaded models, freeing memory \
                          without stopping the process.",
            inputs: vec![required_string("id", "The [[mlx.server]] block id.")],
            outputs: vec![json_output("unloaded", "Confirmation payload.")],
        },
        other => panic!("unknown mlx controller function: {other}"),
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);
        let servers = service.mlx.status_all(&config, &service.http).await;
        let (used, budget) = service.mlx.memory_summary(&config).await;

        to_json(RpcOutcome::new(
            MlxStatusReply {
                enabled: config.mlx.enabled,
                embeddings_backend: config.mlx.embeddings_backend.clone(),
                memory_used_gib: used,
                memory_budget_gib: budget,
                problems: config.mlx.validate(),
                servers,
            },
            Vec::new(),
        ))
    })
}

fn handle_start(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ServerIdParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);
        let status = service
            .mlx
            .start(&config, &service.http, p.id.trim())
            .await?;
        to_json(RpcOutcome::single_log(
            status,
            format!("started MLX server `{}`", p.id.trim()),
        ))
    })
}

fn handle_stop(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ServerIdParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);
        // Report what actually happened. Claiming a stop that did not occur
        // leaves the caller believing memory was freed when the model is
        // still resident.
        let stopped = service.mlx.stop(&config, p.id.trim()).await;
        to_json(RpcOutcome::single_log(
            serde_json::json!({ "id": p.id.trim(), "stopped": stopped }),
            if stopped {
                format!("stopped MLX server `{}`", p.id.trim())
            } else {
                format!("MLX server `{}` was not running", p.id.trim())
            },
        ))
    })
}

fn handle_restart(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ServerIdParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);
        let status = service
            .mlx
            .restart(&config, &service.http, p.id.trim())
            .await?;
        to_json(RpcOutcome::single_log(
            status,
            format!("restarted MLX server `{}`", p.id.trim()),
        ))
    })
}

fn handle_logs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<LogsParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);
        let limit = p.limit.unwrap_or(DEFAULT_LOG_LIMIT).min(MAX_LOG_LIMIT);
        let logs = service.mlx.logs(p.id.trim(), limit).await;
        to_json(RpcOutcome::new(
            serde_json::json!({ "id": p.id.trim(), "lines": logs }),
            Vec::new(),
        ))
    })
}

fn handle_unload(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ServerIdParams>(params)?;
        let config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);
        let id = p.id.trim();

        let base_url = service
            .mlx
            .resolved_base_url(&config, id)
            .await
            .ok_or_else(|| {
                format!(
                    "cannot reach server `{id}`: it is not running in this process and its                      [[mlx.server]] block has no fixed port, so its address is unknown."
                )
            })?;

        // `/unload` sits at the server root, not under `/v1`.
        let root = base_url.trim_end_matches('/').trim_end_matches("/v1");
        let response = service
            .http
            .post(format!("{root}/unload"))
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|err| format!("unload request to `{id}` failed: {err}"))?;

        if !response.status().is_success() {
            return Err(format!(
                "server `{id}` refused the unload: HTTP {}",
                response.status()
            ));
        }

        to_json(RpcOutcome::single_log(
            serde_json::json!({ "id": id, "unloaded": true }),
            format!("unloaded models on MLX server `{id}`"),
        ))
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "mlx_schemas_tests.rs"]
mod tests;
