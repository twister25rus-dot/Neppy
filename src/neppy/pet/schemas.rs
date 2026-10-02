//! Controller schemas and handlers for the `pet` RPC namespace
//! (`openhuman.pet_<fn>`). Handlers reload config per call and delegate to
//! [`super::ops`]; results are bare JSON values.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::neppy::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::ops;
use super::types::PetProfilePatch;

const FUNCTIONS: &[&str] = &[
    "get",
    "update",
    "goal_add",
    "goal_remove",
    "run_now",
    "feed",
    "notes_list",
    "note_dismiss",
    "inbox_list",
    "proposal_decide",
    "digest_now",
];

/// Research-pass controllers, then the desktop companion's
/// (`openhuman.pet_companion_*`, see `companion::runtime::schemas`).
pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS
        .iter()
        .map(|f| schemas(f))
        .chain(super::companion::runtime::all_companion_controller_schemas())
        .collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    FUNCTIONS
        .iter()
        .map(|f| RegisteredController {
            schema: schemas(f),
            handler: handler_for(f),
        })
        .chain(super::companion::runtime::all_companion_registered_controllers())
        .collect()
}

fn handler_for(function: &str) -> fn(Map<String, Value>) -> ControllerFuture {
    match function {
        "get" => handle_get,
        "update" => handle_update,
        "goal_add" => handle_goal_add,
        "goal_remove" => handle_goal_remove,
        "run_now" => handle_run_now,
        "feed" => handle_feed,
        "notes_list" => handle_notes_list,
        "note_dismiss" => handle_note_dismiss,
        "inbox_list" => handle_inbox_list,
        "proposal_decide" => handle_proposal_decide,
        _ => handle_digest_now,
    }
}

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

fn out(comment: &'static str) -> Vec<FieldSchema> {
    vec![field("result", TypeSchema::Json, comment, true)]
}

pub fn schemas(function: &str) -> ControllerSchema {
    let (function, description, inputs, outputs): (&'static str, &'static str, _, _) =
        match function {
            "get" => (
                "get",
                "Get the Pet profile (lazily creates the disabled primary pet).",
                vec![],
                out("PetProfile."),
            ),
            "update" => (
                "update",
                "Update the Pet profile and sync its research job.",
                vec![field(
                    "patch",
                    TypeSchema::Json,
                    "PetProfilePatch: name, persona, enabled, research_preset, digest_time, \
                     quiet_start, quiet_end, notify_budget_per_day, sources.",
                    true,
                )],
                out("PetProfile."),
            ),
            "goal_add" => (
                "goal_add",
                "Add a user-authored goal for the Pet's research.",
                vec![field(
                    "text",
                    TypeSchema::String,
                    "Goal text (1..280 chars).",
                    true,
                )],
                out("PetGoal."),
            ),
            "goal_remove" => (
                "goal_remove",
                "Archive a Pet goal.",
                vec![field("goal_id", TypeSchema::String, "Goal id.", true)],
                out("{ removed: boolean }."),
            ),
            "run_now" => (
                "run_now",
                "Run one Pet research pass now (allowed while disabled).",
                vec![field(
                    "wait",
                    opt(TypeSchema::Bool),
                    "Wait for the pass to finish (default false).",
                    false,
                )],
                out("PetRunSummary."),
            ),
            "feed" => (
                "feed",
                "Recent digests, surfaced notes and the last run.",
                vec![
                    field(
                        "limit",
                        opt(TypeSchema::U64),
                        "Notes limit 1..200 (default 50).",
                        false,
                    ),
                    field(
                        "before",
                        opt(TypeSchema::String),
                        "RFC3339 created_at cursor.",
                        false,
                    ),
                ],
                out("PetFeed."),
            ),
            "notes_list" => (
                "notes_list",
                "List Pet notes, newest first.",
                vec![
                    field(
                        "state",
                        opt(TypeSchema::Enum {
                            variants: vec![
                                "new",
                                "notified",
                                "queued",
                                "digested",
                                "dropped",
                                "dismissed",
                            ],
                        }),
                        "Filter by state.",
                        false,
                    ),
                    field("limit", opt(TypeSchema::U64), "1..200 (default 50).", false),
                    field(
                        "before",
                        opt(TypeSchema::String),
                        "RFC3339 created_at cursor.",
                        false,
                    ),
                ],
                out("PetNote[]."),
            ),
            "note_dismiss" => (
                "note_dismiss",
                "Dismiss a Pet note (and its pending proposal).",
                vec![field("note_id", TypeSchema::String, "Note id.", true)],
                out("PetNote."),
            ),
            "inbox_list" => (
                "inbox_list",
                "Pending Pet proposals and background approvals without a chat card.",
                vec![],
                out("PetInbox."),
            ),
            "proposal_decide" => (
                "proposal_decide",
                "Accept (returns a chat prompt to review; nothing runs) or dismiss a proposal.",
                vec![
                    field("proposal_id", TypeSchema::String, "Proposal id.", true),
                    field(
                        "decision",
                        TypeSchema::Enum {
                            variants: vec!["accept", "dismiss"],
                        },
                        "accept | dismiss.",
                        true,
                    ),
                ],
                out("{ proposal: PetProposal, chat_prompt: string | null }."),
            ),
            "digest_now" => (
                "digest_now",
                "Build a digest from queued and undigested notes now.",
                vec![],
                out("PetDigest | null."),
            ),
            _ => (
                "unknown",
                "Unknown pet controller function.",
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
        namespace: "pet",
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

fn read_opt_str<'a>(params: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("invalid '{key}': expected a string")),
    }
}

fn read_opt_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected an unsigned integer")),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

macro_rules! handler {
    ($name:ident, $fn_name:literal, |$config:ident, $params:ident| $body:expr) => {
        fn $name($params: Map<String, Value>) -> ControllerFuture {
            Box::pin(async move {
                log::debug!(concat!("[rpc:pet_", $fn_name, "] entry"));
                let $config = config_rpc::load_config_with_timeout().await?;
                let result: Result<Value, String> = async { $body }.await;
                log::debug!(
                    concat!("[rpc:pet_", $fn_name, "] exit ok={}"),
                    result.is_ok()
                );
                result
            })
        }
    };
}

handler!(handle_get, "get", |config, _params| to_json(
    ops::pet_get(&config).await?
));

handler!(handle_update, "update", |config, params| {
    let patch: PetProfilePatch = read_required(&params, "patch")?;
    to_json(ops::pet_update(&config, patch).await?)
});

handler!(handle_goal_add, "goal_add", |config, params| {
    let text: String = read_required(&params, "text")?;
    to_json(ops::pet_goal_add(&config, &text).await?)
});

handler!(handle_goal_remove, "goal_remove", |config, params| {
    let goal_id: String = read_required(&params, "goal_id")?;
    to_json(ops::pet_goal_remove(&config, &goal_id).await?)
});

handler!(handle_run_now, "run_now", |config, params| {
    let wait = match params.get("wait") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("invalid 'wait': expected a boolean".into()),
    };
    to_json(ops::pet_run_now(&config, wait).await?)
});

handler!(handle_feed, "feed", |config, params| {
    let limit = read_opt_u64(&params, "limit")?;
    let before = read_opt_str(&params, "before")?;
    to_json(ops::pet_feed(&config, limit, before).await?)
});

handler!(handle_notes_list, "notes_list", |config, params| {
    let state = read_opt_str(&params, "state")?;
    let limit = read_opt_u64(&params, "limit")?;
    let before = read_opt_str(&params, "before")?;
    to_json(ops::pet_notes_list(&config, state, limit, before).await?)
});

handler!(handle_note_dismiss, "note_dismiss", |config, params| {
    let note_id: String = read_required(&params, "note_id")?;
    to_json(ops::pet_note_dismiss(&config, &note_id).await?)
});

handler!(handle_inbox_list, "inbox_list", |config, _params| to_json(
    ops::pet_inbox_list(&config).await?
));

handler!(
    handle_proposal_decide,
    "proposal_decide",
    |config, params| {
        let proposal_id: String = read_required(&params, "proposal_id")?;
        let decision: String = read_required(&params, "decision")?;
        to_json(ops::pet_proposal_decide(&config, &proposal_id, &decision).await?)
    }
);

handler!(handle_digest_now, "digest_now", |config, _params| to_json(
    ops::pet_digest_now(&config).await?
));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_function_has_a_pet_schema_and_handler() {
        let all = all_controller_schemas();
        let companion = super::super::companion::runtime::schemas::FUNCTIONS;
        assert_eq!(all.len(), FUNCTIONS.len() + companion.len());
        for (s, f) in all.iter().zip(FUNCTIONS.iter().chain(companion)) {
            assert_eq!(s.namespace, "pet");
            assert_eq!(s.function, *f);
        }
        assert_eq!(
            all_registered_controllers().len(),
            FUNCTIONS.len() + companion.len()
        );
        assert_eq!(schemas("bogus").function, "unknown");
    }

    #[test]
    fn read_helpers_report_field_named_errors() {
        let mut params = Map::new();
        params.insert("limit".into(), Value::from(-1));
        params.insert("before".into(), Value::from(3));
        assert!(read_opt_u64(&params, "limit")
            .unwrap_err()
            .starts_with("invalid 'limit'"));
        assert!(read_opt_str(&params, "before")
            .unwrap_err()
            .starts_with("invalid 'before'"));
        assert!(read_required::<String>(&params, "note_id")
            .unwrap_err()
            .contains("missing required param 'note_id'"));
        params.insert("patch".into(), serde_json::json!({ "nope": 1 }));
        assert!(read_required::<PetProfilePatch>(&params, "patch")
            .unwrap_err()
            .starts_with("invalid 'patch'"));
    }
}
