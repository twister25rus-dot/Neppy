//! Controller schemas + handlers for the local-install RPCs:
//! `neppy.debug_mode_install_local_{build,status,apply,result}`. Same style as
//! [`super::candidate_schemas`]; chained into the namespace by `schemas.rs`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::neppy::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::local_install;
use super::ops::DebugCtx;

const FUNCTIONS: &[&str] = &[
    "install_local_build",
    "install_local_status",
    "install_local_apply",
    "install_local_result",
];

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
}

fn opt(ty: TypeSchema) -> TypeSchema {
    TypeSchema::Option(Box::new(ty))
}

pub fn controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

pub fn registered_controllers() -> Vec<RegisteredController> {
    FUNCTIONS
        .iter()
        .map(|f| RegisteredController {
            schema: schemas(f),
            handler: match *f {
                "install_local_build" => handle_build,
                "install_local_status" => handle_status,
                "install_local_apply" => handle_apply,
                _ => handle_result,
            },
        })
        .collect()
}

pub fn schemas(function: &str) -> ControllerSchema {
    let result = |comment: &'static str| vec![field("result", TypeSchema::Json, comment, true)];
    let root = || {
        field(
            "project_root",
            opt(TypeSchema::String),
            "Project root (defaults like the other debug_mode calls).",
            false,
        )
    };
    let (function, description, inputs, outputs): (&'static str, &'static str, _, _) =
        match function {
            "install_local_build" => (
                "install_local_build",
                "Build the current source into a Neppy.app in the background (private target dir, \
             no signing key, nothing uploaded). Refuses while a build runs, and when the \
             active debug task changed critical files with no passed candidate.",
                vec![root()],
                result("LocalInstallRecord (phase `building`)."),
            ),
            "install_local_status" => (
                "install_local_status",
                "Phase (idle|building|ready|failed|installing), build log tail and bundle path.",
                vec![],
                result("LocalInstallRecord."),
            ),
            "install_local_apply" => (
                "install_local_apply",
                "Hand the built bundle to the detached installer helper. Only when phase is ready \
             and confirm is true. The caller must then quit the app: the helper waits for it \
             to exit, backs up the installed app, swaps in the new one, relaunches, and \
             restores the backup if the new app does not reach ready.",
                vec![
                    field("confirm", TypeSchema::Bool, "Must be true.", true),
                    root(),
                ],
                result("LocalInstallRecord (phase `installing`)."),
            ),
            "install_local_result" => (
                "install_local_result",
                "The installer's verdict from its last run (installed|restored|failed), or null.",
                vec![field(
                    "acknowledge",
                    opt(TypeSchema::Bool),
                    "True marks the result as seen (one-time notice).",
                    false,
                )],
                result("LocalInstallResult | null."),
            ),
            _ => (
                "unknown",
                "Unknown debug_mode local-install function.",
                vec![field(
                    "function",
                    TypeSchema::String,
                    "Unknown function.",
                    true,
                )],
                vec![field(
                    "error",
                    TypeSchema::String,
                    "Lookup error details.",
                    true,
                )],
            ),
        };
    ControllerSchema {
        namespace: "debug_mode",
        function,
        description,
        inputs,
        outputs,
    }
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

handler!(handle_build, "install_local_build", |ctx, params| {
    let root: Option<String> = read_opt(&params, "project_root")?;
    to_json(local_install::start(&ctx, root.as_deref()).await?)
});

handler!(handle_status, "install_local_status", |ctx, _params| {
    to_json(local_install::status(&ctx).await?)
});

handler!(handle_apply, "install_local_apply", |ctx, params| {
    let confirm: bool = read_opt(&params, "confirm")?.unwrap_or(false);
    let root: Option<String> = read_opt(&params, "project_root")?;
    to_json(local_install::apply(&ctx, confirm, root.as_deref()).await?)
});

handler!(handle_result, "install_local_result", |_ctx, params| {
    let ack: bool = read_opt(&params, "acknowledge")?.unwrap_or(false);
    to_json(local_install::result(ack).await?)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_local_install_function_has_a_schema_and_handler() {
        let all = controller_schemas();
        assert_eq!(all.len(), FUNCTIONS.len());
        for (s, f) in all.iter().zip(FUNCTIONS) {
            assert_eq!(s.namespace, "debug_mode");
            assert_eq!(s.function, *f);
        }
        assert_eq!(registered_controllers().len(), FUNCTIONS.len());
        assert_eq!(schemas("bogus").function, "unknown");
        assert_eq!(
            crate::core::all::rpc_method_name(&schemas("install_local_apply")),
            "neppy.debug_mode_install_local_apply"
        );
    }
}
