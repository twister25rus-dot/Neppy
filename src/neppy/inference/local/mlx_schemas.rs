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
struct UpdateServerParams {
    id: String,
    /// Fields to change. Absent means "leave alone", so the panel can save one
    /// field without resending the rest.
    #[serde(default)]
    patch: super::mlx_patch::ServerPatch,
}

#[derive(Debug, Deserialize)]
struct ModelIdParams {
    model_id: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsBackendParams {
    backend: String,
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
        schemas("update_server"),
        schemas("set_embeddings_backend"),
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
            schema: schemas("update_server"),
            handler: handle_update_server,
        },
        RegisteredController {
            schema: schemas("set_embeddings_backend"),
            handler: handle_set_embeddings_backend,
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
        "update_server" => ControllerSchema {
            namespace: "mlx",
            function: "update_server",
            description: "Change any subset of a server's settings: which models occupy its \
                          slots, and every tuning flag. Absent fields are left alone. Restarts \
                          the server when a changed field is part of its command line.",
            inputs: vec![
                required_string("id", "The [[mlx.server]] block id."),
                json_input(
                    "patch",
                    "Fields to change, e.g. {\"stt_model\": \"mlx-community/whisper-large-v3\", \
                     \"max_tokens\": 1024}. Unknown field names are rejected.",
                ),
            ],
            outputs: vec![json_output(
                "server",
                "What changed, and the status afterwards.",
            )],
        },
        "set_embeddings_backend" => ControllerSchema {
            namespace: "mlx",
            function: "set_embeddings_backend",
            description: "Choose which backend serves embeddings while MLX is the runtime: \
                          `ollama` (the default) or `mlx`. Ollama is not removed either way.",
            inputs: vec![required_string("backend", "`ollama` or `mlx`.")],
            outputs: vec![json_output("embeddings", "The backend now in effect.")],
        },
        other => panic!("unknown mlx controller function: {other}"),
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let (config, service) = config_and_service().await?;
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
        let (config, service) = config_and_service().await?;
        let id = p.id.trim().to_string();
        let status = service.mlx.start(&config, &service.http, &id).await?;

        // Two durable consequences of a successful start, written in one save:
        // pin an auto-assigned port, and register the server as a provider.
        //
        // The port matters because an assigned one is otherwise known only to
        // this process, so a core restart leaves a running server nothing can
        // address. The provider entry matters because a started server that no
        // model picker lists is not actually usable — which is exactly how the
        // first version of this shipped.
        {
            let mut persisted = config.clone();
            let mut dirty = false;

            if let Some(port) = service.mlx.assigned_port(&id).await {
                if let Some(server) = persisted.mlx.servers.iter_mut().find(|s| s.id == id) {
                    if server.port != port {
                        server.port = port;
                        dirty = true;
                    }
                }
            }

            if let Some(server) = persisted.mlx.server(&id).cloned() {
                let base_url = service
                    .mlx
                    .resolved_base_url(&persisted, &id)
                    .await
                    .unwrap_or_else(|| server.base_url(server.port));
                if crate::neppy::inference::local::mlx::upsert_provider_entry(
                    &mut persisted,
                    &server,
                    &base_url,
                ) {
                    dirty = true;
                }
                // Speech is a separate provider list, and the server only
                // belongs in it while a speech slot is actually filled.
                if crate::neppy::inference::local::mlx::upsert_voice_provider_entry(
                    &mut persisted,
                    &server,
                    &base_url,
                ) {
                    dirty = true;
                }
            }

            if dirty {
                if let Err(err) = persisted.save().await {
                    // The server is up and serving; only its registration is
                    // not durable, so this degrades rather than fails.
                    log::warn!("[mlx] could not persist start-time config for `{id}`: {err}");
                } else {
                    log::info!("[mlx] `{id}` registered as a provider and pinned to its port");
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
        let (config, service) = config_and_service().await?;
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
        let (config, service) = config_and_service().await?;
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
        // Logs live in the pool's ring buffer, so nothing here reads config —
        // but resolving the service still requires loading it.
        let (_config, service) = config_and_service().await?;
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
        let (config, service) = config_and_service().await?;
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

fn handle_update_server(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<UpdateServerParams>(params)?;
        let id = p.id.trim().to_string();

        let (mut config, service) = config_and_service().await?;
        let outcome = {
            let server = config
                .mlx
                .servers
                .iter_mut()
                .find(|server| server.id == id)
                .ok_or_else(|| format!("no [[mlx.server]] block with id `{id}`"))?;
            super::mlx_patch::apply(server, p.patch)
        };

        // Saving and restarting on a no-op edit would bounce a loaded model
        // for nothing, and a 27B reload costs minutes.
        if outcome.is_empty() {
            let status = service.mlx.status_of(&config, &service.http, &id).await;
            return to_json(RpcOutcome::new(
                serde_json::json!({ "id": id, "changed": [], "restarted": false, "server": status }),
                Vec::new(),
            ));
        }

        config.save().await.map_err(|err| err.to_string())?;

        // Command-line settings only take effect at spawn, so a running server
        // has to come back. A stopped one stays stopped: editing settings is
        // not a request to start it.
        let running = service.mlx.base_url_for(&id).await.is_some();
        let restart = running && outcome.needs_restart;
        let status = if restart {
            Some(service.mlx.restart(&config, &service.http, &id).await?)
        } else {
            service.mlx.status_of(&config, &service.http, &id).await
        };

        to_json(RpcOutcome::single_log(
            serde_json::json!({
                "id": id,
                "changed": outcome.changed,
                "restarted": restart,
                "server": status,
            }),
            format!("updated `{id}`: {}", outcome.changed.join(", ")),
        ))
    })
}

fn handle_set_embeddings_backend(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<EmbeddingsBackendParams>(params)?;
        let backend = p.backend.trim().to_ascii_lowercase();
        if backend != "ollama" && backend != "mlx" {
            return Err(format!(
                "unknown embeddings backend `{backend}`: expected `ollama` or `mlx`"
            ));
        }

        let mut config = config_rpc::load_config_with_timeout().await?;
        if config.mlx.embeddings_backend == backend {
            return to_json(RpcOutcome::new(
                serde_json::json!({ "backend": backend, "changed": false }),
                Vec::new(),
            ));
        }

        // Switching to MLX without a 1024-dim embedder would leave the memory
        // tree unable to read its own vectors, so say so rather than let it
        // fail later at recall time.
        let problems = if backend == "mlx" {
            let has_embedder = config
                .mlx
                .servers
                .iter()
                .any(|server| server.is_vlm() && !server.embedding_model.trim().is_empty());
            if !has_embedder {
                vec!["No MLX server has an embedding model selected yet.".to_string()]
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        config.mlx.embeddings_backend = backend.clone();
        config.save().await.map_err(|err| err.to_string())?;

        to_json(RpcOutcome::single_log(
            serde_json::json!({ "backend": backend, "changed": true, "problems": problems }),
            format!("embeddings now served by {backend}"),
        ))
    })
}

async fn config_and_service() -> Result<
    (
        crate::neppy::config::Config,
        std::sync::Arc<super::LocalAiService>,
    ),
    String,
> {
    let config = config_rpc::load_config_with_timeout().await?;
    let service = super::global(&config);
    Ok((config, service))
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

fn json_input(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
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
