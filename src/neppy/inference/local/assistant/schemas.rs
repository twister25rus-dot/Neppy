//! RPC surface — namespace `local_assistant`.
//!
//! Handlers parse params and call `api`; the logic lives there. The
//! namespace is added to the registry by the host (`core::all`), under
//! `DomainGroup::Inference`.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::api;
use super::ops::{prod_ctx, stop_worker_when_idle};
use super::types::TaskSpec;

const FUNCTIONS: &[&str] = &[
    "start_task",
    "status",
    "list",
    "resume",
    "cancel",
    "set_enabled",
    "index_refresh",
    "index_status",
    "search",
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("start_task"),
            handler: handle_start_task,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("resume"),
            handler: handle_resume,
        },
        RegisteredController {
            schema: schemas("cancel"),
            handler: handle_cancel,
        },
        RegisteredController {
            schema: schemas("set_enabled"),
            handler: handle_set_enabled,
        },
        RegisteredController {
            schema: schemas("index_refresh"),
            handler: handle_index_refresh,
        },
        RegisteredController {
            schema: schemas("index_status"),
            handler: handle_index_status,
        },
        RegisteredController {
            schema: schemas("search"),
            handler: handle_search,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    let def = |function: &'static str,
               description: &'static str,
               inputs: Vec<FieldSchema>,
               out: (&'static str, &'static str)| ControllerSchema {
        namespace: "local_assistant",
        function,
        description,
        inputs,
        outputs: vec![json_output(out.0, out.1)],
    };
    match function {
        "start_task" => def(
            "start_task",
            "Queue a bounded, resumable task over a project on disk. The model runs in short \
             steps; each step's plan, edits and test result are checkpointed. Edits are applied \
             only when allow_edits is true and the autonomy tier permits; the test command runs \
             only if you supply it here.",
            vec![
                required_string("project_root", "Absolute path of the project directory."),
                required_string("goal", "What the task should accomplish."),
                optional_bool("allow_edits", "Apply the model's edits. Defaults to false."),
                optional_string(
                    "test_command",
                    "Command (run with `sh -c` in project_root) the model may ask to run after edits.",
                ),
                optional_u64("max_steps", "Fewer steps than the configured limit."),
            ],
            ("task", "The queued task record."),
        ),
        "status" => def(
            "status",
            "A task's record, its latest step and that step's effects, and the queue state.",
            vec![required_string("task_id", "Task id.")],
            ("status", "Task detail."),
        ),
        "list" => def(
            "list",
            "Recent tasks, newest first.",
            vec![optional_u64("limit", "Default 20, at most 100.")],
            ("tasks", "Task records."),
        ),
        "resume" => def(
            "resume",
            "Queue a paused, interrupted or failed task to continue from its last checkpoint. \
             Completed edits and test runs are not repeated.",
            vec![required_string("task_id", "Task id.")],
            ("task", "The queued task record."),
        ),
        "cancel" => def(
            "cancel",
            "Cancel a task. A running model call is aborted and the step's checkpoint is kept.",
            vec![required_string("task_id", "Task id.")],
            ("task", "The task record."),
        ),
        "set_enabled" => def(
            "set_enabled",
            "false: finish or checkpoint the current step, accept no new work, and stop the \
             worker. true: resume paused tasks.",
            vec![required_bool("enabled", "Master switch.")],
            ("enabled", "The new state and how many paused tasks were re-queued."),
        ),
        "index_refresh" => def(
            "index_refresh",
            "Bring a project's index up to date (incremental) and report what changed.",
            vec![required_string("project_root", "Absolute path of the project directory.")],
            ("stats", "Refresh statistics."),
        ),
        "index_status" => def(
            "index_status",
            "Size and freshness of a project's index.",
            vec![required_string("project_root", "Absolute path of the project directory.")],
            ("status", "Index status."),
        ),
        "search" => def(
            "search",
            "Debug: the snippets retrieval would select for a query. At most 20.",
            vec![
                required_string("project_root", "Absolute path of the project directory."),
                required_string("query", "Identifiers or words to look for."),
                optional_u64("limit", "At most 20."),
            ],
            ("snippets", "Snippets with path, line range and why they matched."),
        ),
        other => panic!("unknown local_assistant controller function: {other}"),
    }
}

#[derive(Deserialize)]
struct StartParams {
    project_root: PathBuf,
    goal: String,
    #[serde(default)]
    allow_edits: bool,
    #[serde(default)]
    test_command: Option<String>,
    #[serde(default)]
    max_steps: Option<u32>,
}

#[derive(Deserialize)]
struct TaskIdParams {
    task_id: String,
}

#[derive(Deserialize)]
struct ListParams {
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct EnabledParams {
    enabled: bool,
}

#[derive(Deserialize)]
struct RootParams {
    project_root: PathBuf,
}

#[derive(Deserialize)]
struct SearchParams {
    project_root: PathBuf,
    query: String,
    #[serde(default)]
    limit: Option<usize>,
}

fn parse<T: serde::de::DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn reply<T: serde::Serialize>(value: T, log: String) -> Result<Value, String> {
    RpcOutcome::single_log(value, log).into_cli_compatible_json()
}

fn err<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

fn handle_start_task(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: StartParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let task = api::start_task(
            &ctx,
            TaskSpec {
                project_root: p.project_root,
                goal: p.goal,
                allow_edits: p.allow_edits,
                test_command: p.test_command,
                max_steps: p.max_steps,
            },
        )
        .map_err(err)?;
        let log = format!("task {} queued", task.id);
        reply(task, log)
    })
}

fn handle_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: TaskIdParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let detail = api::task_status(&ctx, &p.task_id).map_err(err)?;
        reply(detail, format!("status of task {}", p.task_id))
    })
}

fn handle_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: ListParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let tasks = api::list_tasks(&ctx, p.limit).map_err(err)?;
        let log = format!("{} task(s)", tasks.len());
        reply(tasks, log)
    })
}

fn handle_resume(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: TaskIdParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let task = api::resume_task(&ctx, &p.task_id).map_err(err)?;
        reply(task, format!("task {} queued to resume", p.task_id))
    })
}

fn handle_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: TaskIdParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let task = api::cancel_task(&ctx, &p.task_id).map_err(err)?;
        reply(task, format!("task {} cancel requested", p.task_id))
    })
}

fn handle_set_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: EnabledParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let result = api::set_enabled(&ctx, p.enabled).map_err(err)?;
        if !p.enabled {
            stop_worker_when_idle(std::sync::Arc::clone(&ctx.controller));
        }
        reply(result, format!("local assistant enabled={}", p.enabled))
    })
}

fn handle_index_refresh(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: RootParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let stats = api::index_refresh(&ctx, &p.project_root)
            .await
            .map_err(err)?;
        reply(stats, "index refreshed".to_string())
    })
}

fn handle_index_status(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: RootParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let status = api::index_status(&ctx, &p.project_root).map_err(err)?;
        reply(status, "index status".to_string())
    })
}

fn handle_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p: SearchParams = parse(params)?;
        let ctx = prod_ctx().await?;
        let snippets = api::search(&ctx, &p.project_root, &p.query, p.limit).map_err(err)?;
        let log = format!("{} snippet(s)", snippets.len());
        reply(snippets, log)
    })
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

fn optional_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment,
        required: false,
    }
}

fn required_bool(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Bool,
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

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
