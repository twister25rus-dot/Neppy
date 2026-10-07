//! Controller schemas + handlers for the staged-update candidate RPCs:
//! `neppy.debug_mode_candidate_{start,status,cancel}`. Same style as
//! [`super::schemas`]; the lead chains these into the namespace in `mod.rs`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::neppy::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::candidate;
use super::ops::DebugCtx;

const FUNCTIONS: &[&str] = &["candidate_start", "candidate_status", "candidate_cancel"];

fn field(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: false,
    }
}

fn opt_string() -> TypeSchema {
    TypeSchema::Option(Box::new(TypeSchema::String))
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
                "candidate_start" => handle_start,
                "candidate_status" => handle_status,
                _ => handle_cancel,
            },
        })
        .collect()
}

pub fn schemas(function: &str) -> ControllerSchema {
    let result = |comment: &'static str| {
        vec![FieldSchema {
            name: "result",
            ty: TypeSchema::Json,
            comment,
            required: true,
        }]
    };
    let (function, description, inputs, outputs): (&'static str, &'static str, _, _) =
        match function {
            "candidate_start" => (
                "candidate_start",
                "Build the modified source into an isolated target dir, launch it as a \
                 separate process and health-check it, in the background. Refuses while \
                 another candidate runs. Never touches the running app.",
                vec![
                    field(
                        "task_id",
                        opt_string(),
                        "Debug task the candidate validates.",
                    ),
                    field(
                        "project_root",
                        opt_string(),
                        "Project root (defaults like the other debug_mode calls).",
                    ),
                ],
                result("CandidateRecord (phase `building`)."),
            ),
            "candidate_status" => (
                "candidate_status",
                "A candidate record by id, or the latest one (null when none exist).",
                vec![field(
                    "candidate_id",
                    opt_string(),
                    "Defaults to the latest candidate.",
                )],
                result("CandidateRecord | null."),
            ),
            "candidate_cancel" => (
                "candidate_cancel",
                "Cancel the running candidate (kills its build and process group).",
                vec![],
                result("The cancelled CandidateRecord."),
            ),
            _ => (
                "unknown",
                "Unknown debug_mode candidate function.",
                vec![FieldSchema {
                    name: "function",
                    ty: TypeSchema::String,
                    comment: "Unknown function.",
                    required: true,
                }],
                vec![FieldSchema {
                    name: "error",
                    ty: TypeSchema::String,
                    comment: "Lookup error details.",
                    required: true,
                }],
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
                let $ctx = DebugCtx::new(&config.workspace_dir);
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

handler!(handle_start, "candidate_start", |ctx, params| {
    let task: Option<String> = read_opt(&params, "task_id")?;
    let root: Option<String> = read_opt(&params, "project_root")?;
    to_json(candidate::start(&ctx, root.as_deref(), task.as_deref()).await?)
});

handler!(handle_status, "candidate_status", |ctx, params| {
    let id: Option<String> = read_opt(&params, "candidate_id")?;
    to_json(candidate::status(&ctx, id.as_deref()).await?)
});

handler!(handle_cancel, "candidate_cancel", |ctx, _params| {
    to_json(candidate::cancel(&ctx).await?)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_candidate_function_has_a_schema_and_handler() {
        let all = controller_schemas();
        assert_eq!(all.len(), FUNCTIONS.len());
        for (s, f) in all.iter().zip(FUNCTIONS) {
            assert_eq!(s.namespace, "debug_mode");
            assert_eq!(s.function, *f);
        }
        assert_eq!(registered_controllers().len(), FUNCTIONS.len());
        assert_eq!(schemas("bogus").function, "unknown");
        assert_eq!(
            crate::core::all::rpc_method_name(&schemas("candidate_start")),
            "neppy.debug_mode_candidate_start"
        );
    }
}
