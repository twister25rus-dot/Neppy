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
struct SetModelParams {
    id: String,
    /// Empty clears the slot, so the server starts with nothing preloaded and
    /// serves whatever a request names.
    #[serde(default)]
    model_id: String,
}

#[derive(Debug, Deserialize)]
struct ModelIdParams {
    model_id: String,
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
    /// What chat currently routes to. Shown so a started server that nothing
    /// uses is visibly distinguishable from one that is actually serving.
    chat_provider: Option<String>,
    /// Whether chat routes to MLX at all.
    chat_uses_mlx: bool,
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
        schemas("models_list"),
        schemas("models_delete"),
        schemas("set_model"),
        schemas("use_for_chat"),
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
        RegisteredController {
            schema: schemas("models_list"),
            handler: handle_models_list,
        },
        RegisteredController {
            schema: schemas("models_delete"),
            handler: handle_models_delete,
        },
        RegisteredController {
            schema: schemas("set_model"),
            handler: handle_set_model,
        },
        RegisteredController {
            schema: schemas("use_for_chat"),
            handler: handle_use_for_chat,
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
        "models_list" => ControllerSchema {
            namespace: "mlx",
            function: "models_list",
            description: "Locally cached models with their on-disk size, largest first.",
            inputs: vec![],
            outputs: vec![json_output("cache", "Cached models and total disk use.")],
        },
        "models_delete" => ControllerSchema {
            namespace: "mlx",
            function: "models_delete",
            description: "Move one cached model to the Trash, reclaiming its disk space. \
                          Recoverable from the Trash until it is emptied.",
            inputs: vec![required_string(
                "model_id",
                "Hugging Face repo id, e.g. mlx-community/Qwen3.8-27B-nvfp4.",
            )],
            outputs: vec![json_output("deleted", "Reclaimed size and confirmation.")],
        },
        "set_model" => ControllerSchema {
            namespace: "mlx",
            function: "set_model",
            description: "Choose which checkpoint a server loads. Persists to config and \
                          restarts the server when it is running, because the model is fixed \
                          at launch.",
            inputs: vec![
                required_string("id", "The [[mlx.server]] block id."),
                optional_string(
                    "model_id",
                    "Hugging Face repo id to load. Empty clears the slot so the server \
                     preloads nothing.",
                ),
            ],
            outputs: vec![json_output("server", "Status after the change.")],
        },
        "use_for_chat" => ControllerSchema {
            namespace: "mlx",
            function: "use_for_chat",
            description: "Route chat, reasoning and vision to this MLX server, pointing the \
                          local runtime at its address.",
            inputs: vec![required_string("id", "The [[mlx.server]] block id.")],
            outputs: vec![json_output("routing", "The routing that was applied.")],
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
                chat_provider: config.chat_provider.clone(),
                chat_uses_mlx: config
                    .chat_provider
                    .as_deref()
                    .is_some_and(|provider| provider.starts_with("mlx:")),
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
        let id = p.id.trim().to_string();
        let status = service.mlx.start(&config, &service.http, &id).await?;

        // Pin an auto-assigned port into config. Without this the address is
        // known only to this process, so a core restart leaves a running
        // server that nothing can address, and `use_for_chat` has nothing to
        // point `local_ai.base_url` at.
        if config
            .mlx
            .server(&id)
            .is_some_and(|server| server.port == 0)
        {
            if let Some(port) = service.mlx.assigned_port(&id).await {
                let mut persisted = config.clone();
                if let Some(server) = persisted
                    .mlx
                    .servers
                    .iter_mut()
                    .find(|server| server.id == id)
                {
                    server.port = port;
                }
                if let Err(err) = persisted.save().await {
                    // The server is up and usable; only the address is not
                    // durable, so this is a warning rather than a failure.
                    log::warn!("[mlx] could not persist the assigned port for `{id}`: {err}");
                } else {
                    log::info!("[mlx] pinned `{id}` to its assigned port {port}");
                }
            }
        }

        to_json(RpcOutcome::single_log(
            status,
            format!("started MLX server `{id}`"),
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

fn handle_models_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // A pure filesystem scan: no config and no running server needed.
        to_json(RpcOutcome::new(
            super::service::mlx_admin::models::list_cached_models(),
            Vec::new(),
        ))
    })
}

fn handle_models_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ModelIdParams>(params)?;
        let model_id = p.model_id.trim().to_string();
        let reclaimed = super::service::mlx_admin::models::delete_cached_model(&model_id)?;
        to_json(RpcOutcome::single_log(
            serde_json::json!({
                "model_id": model_id,
                "reclaimed_gib": reclaimed,
                "recoverable_from_trash": true,
            }),
            format!("moved `{model_id}` to the Trash, reclaiming {reclaimed:.1} GiB"),
        ))
    })
}

fn handle_set_model(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<SetModelParams>(params)?;
        let id = p.id.trim().to_string();
        let model_id = p.model_id.trim().to_string();

        let mut config = config_rpc::load_config_with_timeout().await?;
        {
            let server = config
                .mlx
                .servers
                .iter_mut()
                .find(|server| server.id == id)
                .ok_or_else(|| format!("no [[mlx.server]] block with id `{id}`"))?;
            server.model = model_id.clone();
        }
        config.save().await.map_err(|err| err.to_string())?;

        // The checkpoint is a launch argument, so a running server has to be
        // restarted to pick it up. Restarting one that is stopped would start
        // it, which is not what "choose a model" means, so only restart what
        // was already running.
        let service = super::global(&config);
        let was_running = service.mlx.base_url_for(&id).await.is_some();
        let status = if was_running {
            Some(service.mlx.restart(&config, &service.http, &id).await?)
        } else {
            service.mlx.status_of(&config, &service.http, &id).await
        };

        to_json(RpcOutcome::single_log(
            serde_json::json!({ "id": id, "model_id": model_id, "restarted": was_running, "server": status }),
            if model_id.is_empty() {
                format!("cleared the model slot on `{id}`")
            } else {
                format!("set `{id}` to load {model_id}")
            },
        ))
    })
}

fn handle_use_for_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<ServerIdParams>(params)?;
        let id = p.id.trim().to_string();

        let mut config = config_rpc::load_config_with_timeout().await?;
        let service = super::global(&config);

        // The address has to be pinned into `local_ai.base_url`, because that
        // is what the provider factory reads for an `mlx:` model string. On
        // this machine it pointed at LM Studio, so routing chat to MLX without
        // this would have reached the wrong server entirely.
        let base_url = service
            .mlx
            .resolved_base_url(&config, &id)
            .await
            .ok_or_else(|| {
                format!(
                    "`{id}` has no known address: start it first, or give its                      [[mlx.server]] block a fixed port."
                )
            })?;

        let server = config
            .mlx
            .server(&id)
            .ok_or_else(|| format!("no [[mlx.server]] block with id `{id}`"))?
            .clone();

        if server.model.trim().is_empty() {
            return Err(format!(
                "`{id}` has no model selected. Choose one first, or the chat provider                  would name nothing."
            ));
        }

        let provider_string = format!("mlx:{}", server.model.trim());

        config.local_ai.provider = "mlx".to_string();
        config.local_ai.base_url = Some(base_url.clone());
        config.local_ai.model_id = server.model.trim().to_string();
        config.local_ai.chat_model_id = server.model.trim().to_string();
        config.chat_provider = Some(provider_string.clone());
        config.reasoning_provider = Some(provider_string.clone());
        // Vision only when the checkpoint can actually see; a text model
        // routed here would fail every image turn.
        if server.is_vlm() {
            config.vision_provider = Some(provider_string.clone());
        }
        // Embeddings deliberately untouched: the memory tree is fixed at 1024
        // dims and bge-m3 is what filled it.
        config.save().await.map_err(|err| err.to_string())?;

        to_json(RpcOutcome::single_log(
            serde_json::json!({
                "id": id,
                "base_url": base_url,
                "provider": provider_string,
                "vision": server.is_vlm(),
                "embeddings_unchanged": config.embeddings_provider.clone(),
            }),
            format!("chat now routes to MLX server `{id}` ({provider_string})"),
        ))
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
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
