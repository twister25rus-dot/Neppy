//! Controller schemas + handlers for the release RPCs:
//! `neppy.debug_mode_release_{preflight,start,status}`. Same style as
//! [`super::local_install_schemas`]; chained into the namespace by `schemas.rs`.
//!
//! These are RPC-only on purpose: no agent tool wraps them, so a model cannot
//! publish a release.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::neppy::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::ops::DebugCtx;
use super::release;

const FUNCTIONS: &[&str] = &["release_preflight", "release_start", "release_status"];

fn field(name: &'static str, ty: TypeSchema, comment: &'static str, required: bool) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required,
    }
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
                "release_preflight" => handle_preflight,
                "release_start" => handle_start,
                _ => handle_status,
            },
        })
        .collect()
}

pub fn schemas(function: &str) -> ControllerSchema {
    let result = |comment: &'static str| vec![field("result", TypeSchema::Json, comment, true)];
    let (function, description, inputs, outputs): (&'static str, &'static str, _, _) =
        match function {
            "release_preflight" => (
                "release_preflight",
                "Whether a release can be published now: branch, clean tree, origin not ahead, \
                 commits to release, signing key present, gh authenticated, and the stable \
                 blocker codes that follow from those.",
                vec![],
                result("ReleasePreflight."),
            ),
            "release_start" => (
                "release_start",
                "Run scripts/release-neppy.sh <version> detached (builds and signs, pushes, tags, \
                 creates the GitHub release). The caller must have shown the user this exact \
                 version and received a confirm. Refuses a non-X.Y.Z version, one not above the \
                 current version, any preflight blocker, and a release already running.",
                vec![field(
                    "version",
                    TypeSchema::String,
                    "Version to publish, X.Y.Z.",
                    true,
                )],
                result("ReleaseRecord (phase `running`)."),
            ),
            "release_status" => (
                "release_status",
                "Phase (idle|running|succeeded|failed) of the last release, with a masked log tail.",
                vec![],
                result("ReleaseRecord."),
            ),
            _ => (
                "unknown",
                "Unknown debug_mode release function.",
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

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
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

handler!(handle_preflight, "release_preflight", |ctx, _params| {
    to_json(release::preflight(&ctx).await?)
});

handler!(handle_start, "release_start", |ctx, params| {
    let version: String = read_required(&params, "version")?;
    to_json(release::start(&ctx, &version).await?)
});

handler!(handle_status, "release_status", |ctx, _params| {
    to_json(release::status(&ctx).await?)
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_release_function_has_a_schema_and_handler() {
        let all = controller_schemas();
        assert_eq!(all.len(), FUNCTIONS.len());
        for (s, f) in all.iter().zip(FUNCTIONS) {
            assert_eq!(s.namespace, "debug_mode");
            assert_eq!(s.function, *f);
        }
        assert_eq!(registered_controllers().len(), FUNCTIONS.len());
        assert_eq!(schemas("bogus").function, "unknown");
        assert_eq!(
            crate::core::all::rpc_method_name(&schemas("release_start")),
            "neppy.debug_mode_release_start"
        );
    }

    #[test]
    fn release_start_requires_a_version_param() {
        let err = read_required::<String>(&Map::new(), "version").unwrap_err();
        assert!(err.contains("missing required param 'version'"), "{err}");
        let start = schemas("release_start");
        assert!(start
            .inputs
            .iter()
            .any(|f| f.name == "version" && f.required));
    }
}
