//! Controller schemas and handlers for the `debug_mode` RPC namespace
//! (`neppy.debug_mode_<fn>`). Handlers load the workspace dir per call and
//! delegate to [`super::ops`]; results are bare JSON values.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::ControllerSchema;
use crate::neppy::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::commit;
use super::ops::{self, DebugCtx};
use super::schema_defs::schemas;
use super::settings;
use super::types::TaskPatch;

const FUNCTIONS: &[&str] = &[
    "status",
    "discover_checks",
    "checkpoint_create",
    "checkpoint_list",
    "checkpoint_get",
    "rollback",
    "commit",
    "diff",
    "task_start",
    "task_update",
    "task_list",
    "task_get",
    "run_check",
    "audit_tail",
    "settings_get",
    "settings_update",
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    let mut all: Vec<ControllerSchema> = FUNCTIONS.iter().map(|f| schemas(f)).collect();
    all.extend(super::candidate_schemas::controller_schemas());
    all
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    let mut all: Vec<RegisteredController> = FUNCTIONS
        .iter()
        .map(|f| RegisteredController {
            schema: schemas(f),
            handler: handler_for(f),
        })
        .collect();
    all.extend(super::candidate_schemas::registered_controllers());
    all
}

fn handler_for(function: &str) -> fn(Map<String, Value>) -> ControllerFuture {
    match function {
        "status" => handle_status,
        "discover_checks" => handle_discover_checks,
        "checkpoint_create" => handle_checkpoint_create,
        "checkpoint_list" => handle_checkpoint_list,
        "checkpoint_get" => handle_checkpoint_get,
        "rollback" => handle_rollback,
        "commit" => handle_commit,
        "diff" => handle_diff,
        "task_start" => handle_task_start,
        "task_update" => handle_task_update,
        "task_list" => handle_task_list,
        "task_get" => handle_task_get,
        "run_check" => handle_run_check,
        "settings_get" => handle_settings_get,
        "settings_update" => handle_settings_update,
        _ => handle_audit_tail,
    }
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_opt<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

macro_rules! handler {
    ($name:ident, $fn_name:literal, |$ctx:ident, $params:ident| $body:expr) => {
        fn $name($params: Map<String, Value>) -> ControllerFuture {
            Box::pin(async move {
                log::debug!(concat!("[rpc:debug_mode_", $fn_name, "] entry"));
                let config = config_rpc::load_config_with_timeout().await?;
                let $ctx = DebugCtx::new(&config.workspace_dir).with_settings(&config.debug_mode);
                let result: Result<Value, String> = async { $body }.await;
                log::debug!(
                    concat!("[rpc:debug_mode_", $fn_name, "] exit ok={}"),
                    result.is_ok()
                );
                result
            })
        }
    };
}

handler!(handle_status, "status", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    let mut v = to_json(ops::status(&ctx, root.as_deref()).await?)?;
    // `enabled` rides along without widening `DebugStatus` (types.rs).
    if let Value::Object(m) = &mut v {
        m.insert("enabled".into(), Value::Bool(ctx.settings.enabled));
    }
    Ok(v)
});

handler!(handle_discover_checks, "discover_checks", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    to_json(ops::discover_checks(&ctx, root.as_deref()).await?)
});

handler!(
    handle_checkpoint_create,
    "checkpoint_create",
    |ctx, params| {
        let root: Option<String> = read_opt(&params, "project_root")?;
        let description: String = read_required(&params, "description")?;
        let task_id: Option<String> = read_opt(&params, "task_id")?;
        to_json(
            ops::checkpoint_create(&ctx, root.as_deref(), &description, task_id.as_deref()).await?,
        )
    }
);

handler!(handle_checkpoint_list, "checkpoint_list", |ctx, params| {
    to_json(ops::checkpoint_list(&ctx, read_opt(&params, "limit")?).await?)
});

handler!(handle_checkpoint_get, "checkpoint_get", |ctx, params| {
    let id: String = read_required(&params, "checkpoint_id")?;
    to_json(ops::checkpoint_get(&ctx, &id).await?)
});

handler!(handle_rollback, "rollback", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    let id: String = read_required(&params, "checkpoint_id")?;
    let confirm: bool = read_required(&params, "confirm")?;
    to_json(ops::rollback(&ctx, root.as_deref(), &id, confirm).await?)
});

handler!(handle_commit, "commit", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    let task_id: String = read_required(&params, "task_id")?;
    let message: String = read_required(&params, "message")?;
    let confirm: bool = read_required(&params, "confirm")?;
    to_json(commit::commit(&ctx, root.as_deref(), &task_id, &message, confirm).await?)
});

handler!(handle_diff, "diff", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    let id: Option<String> = read_opt(&params, "checkpoint_id")?;
    to_json(ops::diff(&ctx, root.as_deref(), id.as_deref()).await?)
});

handler!(handle_task_start, "task_start", |ctx, params| {
    let request: String = read_required(&params, "request")?;
    to_json(ops::task_start(&ctx, &request).await?)
});

handler!(handle_task_update, "task_update", |ctx, params| {
    let task_id: String = read_required(&params, "task_id")?;
    let mut patch_params = params.clone();
    patch_params.remove("task_id");
    let patch: TaskPatch = serde_json::from_value(Value::Object(patch_params))
        .map_err(|e| format!("invalid task_update params: {e}"))?;
    to_json(ops::task_update(&ctx, &task_id, patch).await?)
});

handler!(handle_task_list, "task_list", |ctx, params| {
    to_json(ops::task_list(&ctx, read_opt(&params, "limit")?).await?)
});

handler!(handle_task_get, "task_get", |ctx, params| {
    let id: String = read_required(&params, "task_id")?;
    to_json(ops::task_get(&ctx, &id).await?)
});

handler!(handle_run_check, "run_check", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    let check_id: Option<String> = read_opt(&params, "check_id")?;
    let command: Option<Vec<String>> = read_opt(&params, "command")?;
    let timeout: Option<u64> = read_opt(&params, "timeout_secs")?;
    let task_id: Option<String> = read_opt(&params, "task_id")?;
    to_json(
        ops::run_check(
            &ctx,
            root.as_deref(),
            check_id.as_deref(),
            command,
            timeout,
            task_id.as_deref(),
        )
        .await?,
    )
});

handler!(handle_settings_get, "settings_get", |ctx, _params| {
    to_json(settings::get(&ctx).await?)
});

handler!(handle_settings_update, "settings_update", |ctx, params| {
    let patch: Map<String, Value> = read_required(&params, "patch")?;
    to_json(settings::update(&ctx, patch).await?)
});

handler!(handle_audit_tail, "audit_tail", |ctx, params| {
    to_json(ops::audit_tail(&ctx, read_opt(&params, "limit")?).await?)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_function_has_a_schema_and_handler() {
        let all = all_controller_schemas();
        let candidates = super::super::candidate_schemas::controller_schemas().len();
        assert_eq!(all.len(), FUNCTIONS.len() + candidates);
        for (s, f) in all.iter().zip(FUNCTIONS) {
            assert_eq!(s.namespace, "debug_mode");
            assert_eq!(s.function, *f);
        }
        assert!(all.iter().all(|s| s.namespace == "debug_mode"));
        assert_eq!(
            all_registered_controllers().len(),
            FUNCTIONS.len() + candidates
        );
        assert_eq!(schemas("bogus").function, "unknown");
    }

    #[test]
    fn rpc_method_names_follow_namespace_convention() {
        let s = schemas("status");
        assert_eq!(
            crate::core::all::rpc_method_name(&s),
            "neppy.debug_mode_status"
        );
    }

    #[test]
    fn read_helpers_report_field_named_errors() {
        let mut params = Map::new();
        params.insert("limit".into(), Value::from(-1));
        assert!(read_opt::<u64>(&params, "limit")
            .unwrap_err()
            .starts_with("invalid 'limit'"));
        assert!(read_required::<String>(&params, "task_id")
            .unwrap_err()
            .contains("missing required param 'task_id'"));
    }
}
