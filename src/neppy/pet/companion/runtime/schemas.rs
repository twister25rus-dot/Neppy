//! Controller schemas + handlers for `openhuman.pet_companion_*`, chained into
//! the `pet` namespace by `pet::schemas`. Handlers resolve the global runtime
//! and delegate to [`super::ops`]. `lease` is the shell's 2 s heartbeat, so it
//! reloads config at most every 30 s instead of on every call.

use std::time::Duration;

use serde_json::{Map, Value};

use super::ops;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::neppy::config::rpc as config_rpc;

const LEASE_CONFIG_REFRESH: Duration = Duration::from_secs(30);

pub const FUNCTIONS: &[&str] = &[
    "companion_get",
    "companion_update",
    "companion_status",
    "companion_lease",
    "companion_pause",
    "companion_resume",
    "companion_ask",
    "companion_capture",
    "companion_suggestions",
    "companion_suggestion_act",
    "companion_data",
    "companion_data_delete",
    "companion_request_permission",
];

/// Fields of the flat `companion_update` patch.
const PATCH_FIELDS: &[&str] = &[
    "enabled",
    "level",
    "sources",
    "screen_min_interval_secs",
    "allow_cloud_model",
    "retention_days",
    "chattiness",
    "min_interval_min",
    "max_per_hour",
    "app_cooldown_min",
    "category_levels",
    "excluded_apps",
    "excluded_title_patterns",
    "muted_kinds",
    "muted_apps",
    "hotkeys",
    "ocr_languages",
];

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    FUNCTIONS.iter().map(|f| schemas(f)).collect()
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    FUNCTIONS
        .iter()
        .map(|f| RegisteredController {
            schema: schemas(f),
            handler: handler_for(f),
        })
        .collect()
}

fn handler_for(function: &str) -> fn(Map<String, Value>) -> ControllerFuture {
    match function {
        "companion_get" => handle_get,
        "companion_update" => handle_update,
        "companion_status" => handle_status,
        "companion_lease" => handle_lease,
        "companion_pause" => handle_pause,
        "companion_resume" => handle_resume,
        "companion_ask" => handle_ask,
        "companion_capture" => handle_capture,
        "companion_suggestions" => handle_suggestions,
        "companion_suggestion_act" => handle_act,
        "companion_data" => handle_data,
        "companion_data_delete" => handle_data_delete,
        _ => handle_request_permission,
    }
}

fn f(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(ty)),
        comment,
        required: false,
    }
}

fn req(name: &'static str, ty: TypeSchema, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty,
        comment,
        required: true,
    }
}

fn source_field() -> FieldSchema {
    f(
        "source",
        TypeSchema::String,
        "hotkey | tray | ui (logged, never content).",
    )
}

pub fn schemas(function: &str) -> ControllerSchema {
    let (function, description, inputs): (&'static str, &'static str, Vec<FieldSchema>) =
        match function {
            "companion_get" => ("companion_get", "Desktop companion settings (+ unavailable_sources).", vec![]),
            "companion_update" => (
                "companion_update",
                "Apply a flat CompanionSettingsPatch; lists replace, sources/category_levels merge.",
                PATCH_FIELDS
                    .iter()
                    .map(|n| f(n, TypeSchema::Json, "CompanionSettingsPatch field."))
                    .collect(),
            ),
            "companion_status" => ("companion_status", "Companion state, permissions, recent summaries, metrics.", vec![]),
            "companion_lease" => (
                "companion_lease",
                "Shell-only indicator lease (6 s). No visible indicator, no observation.",
                vec![
                    f("indicator", TypeSchema::String, "Indicator kind (tray)."),
                    req("visible", TypeSchema::Bool, "The indicator is visible right now."),
                    f("hotkey_errors", TypeSchema::Json, "[{name, error}] registration failures."),
                ],
            ),
            "companion_pause" => (
                "companion_pause",
                "Pause observation now (optionally for N minutes).",
                vec![f("minutes", TypeSchema::U64, "1..=1440; omitted = until resumed."), source_field()],
            ),
            "companion_resume" => ("companion_resume", "Resume observation.", vec![source_field()]),
            "companion_ask" => ("companion_ask", "Shell-only: ask the Pet about what is on screen.", vec![source_field()]),
            "companion_capture" => (
                "companion_capture",
                "Shell-only: capture a screen region, OCR it on-device (blocks up to 60 s).",
                vec![source_field()],
            ),
            "companion_suggestions" => (
                "companion_suggestions",
                "Suggestions, newest first.",
                vec![
                    f("limit", TypeSchema::U64, "1..=100 (default 50)."),
                    f("before", TypeSchema::String, "RFC3339 created_at cursor."),
                ],
            ),
            "companion_suggestion_act" => (
                "companion_suggestion_act",
                "Act on a suggestion (high-risk categories are always refused).",
                vec![
                    req("id", TypeSchema::String, "Suggestion id."),
                    req("action", TypeSchema::String, "explain|draft|copy_text|save_note|open_chat|prepare_command|handoff|dismiss|mute_kind|mute_app."),
                    f("text", TypeSchema::String, "Edited hand-off prompt (<= 2000 chars)."),
                ],
            ),
            "companion_data" => ("companion_data", "Retained suggestions and the content-free action log.", vec![]),
            "companion_data_delete" => (
                "companion_data_delete",
                "Delete one suggestion or all companion data.",
                vec![
                    f("suggestion_id", TypeSchema::String, "Delete this suggestion."),
                    f("all", TypeSchema::Bool, "Delete everything."),
                    f("include_saved_notes", TypeSchema::Bool, "Also delete Pet notes saved from the companion."),
                ],
            ),
            "companion_request_permission" => (
                "companion_request_permission",
                "Ask the OS for a permission (explicit user click).",
                vec![req("kind", TypeSchema::String, "accessibility | screen_recording.")],
            ),
            _ => ("unknown", "Unknown pet companion function.", vec![]),
        };
    ControllerSchema {
        namespace: "pet",
        function,
        description,
        inputs,
        outputs: vec![FieldSchema {
            name: "result",
            ty: TypeSchema::Json,
            comment: "See the pet companion RPC contract.",
            required: true,
        }],
    }
}

fn opt_str(p: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("invalid '{key}': expected a string")),
    }
}

fn opt_bool(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("invalid '{key}': expected a boolean")),
    }
}

fn opt_u64(p: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected an unsigned integer")),
    }
}

fn source(p: &Map<String, Value>) -> Result<String, String> {
    let s = opt_str(p, "source")?.unwrap_or_else(|| "ui".into());
    match s.as_str() {
        "hotkey" | "tray" | "ui" => Ok(s),
        _ => Err("invalid 'source': expected hotkey|tray|ui".into()),
    }
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

fn rt() -> &'static std::sync::Arc<super::state::Runtime> {
    super::global()
}

handler!(handle_get, "companion_get", |config, _p| ops::get(
    rt(),
    &config
));
handler!(handle_update, "companion_update", |config, p| ops::update(
    rt(),
    &config,
    p
));
handler!(handle_status, "companion_status", |config, _p| ops::status(
    rt(),
    &config
));
handler!(handle_pause, "companion_pause", |config, p| {
    let minutes = opt_u64(&p, "minutes")?
        .map(|m| u32::try_from(m).map_err(|_| "invalid 'minutes': must be 1..=1440".to_string()))
        .transpose()?;
    ops::pause(rt(), &config, minutes, &source(&p)?)
});
handler!(handle_resume, "companion_resume", |config, p| ops::resume(
    rt(),
    &config,
    &source(&p)?
));
handler!(handle_ask, "companion_ask", |config, p| ops::ask(
    rt(),
    &config,
    &source(&p)?
)
.await);
handler!(handle_capture, "companion_capture", |config, p| {
    ops::capture(rt(), &config, &source(&p)?).await
});
handler!(handle_suggestions, "companion_suggestions", |config, p| {
    ops::suggestions(
        rt(),
        &config,
        opt_u64(&p, "limit")?,
        opt_str(&p, "before")?.as_deref(),
    )
});
handler!(handle_act, "companion_suggestion_act", |config, p| {
    let id = opt_str(&p, "id")?.ok_or("missing required param 'id'")?;
    let action = opt_str(&p, "action")?.ok_or("missing required param 'action'")?;
    let text = opt_str(&p, "text")?;
    let r = ops::suggestion_act(rt(), &config, &id, &action, text.as_deref()).await?;
    serde_json::to_value(r).map_err(|e| e.to_string())
});
handler!(handle_data, "companion_data", |config, _p| ops::data(
    rt(),
    &config
));
handler!(handle_data_delete, "companion_data_delete", |config, p| {
    ops::data_delete(
        rt(),
        &config,
        opt_str(&p, "suggestion_id")?.as_deref(),
        opt_bool(&p, "all")?.unwrap_or(false),
        opt_bool(&p, "include_saved_notes")?.unwrap_or(false),
    )
    .await
});
handler!(
    handle_request_permission,
    "companion_request_permission",
    |_config, p| {
        let kind = opt_str(&p, "kind")?.ok_or("missing required param 'kind'")?;
        ops::request_permission(rt(), &kind)
    }
);

/// The shell's heartbeat: no per-call config load once bound.
fn handle_lease(p: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let runtime = rt();
        let stale = runtime
            .config_age()
            .is_none_or(|a| a > LEASE_CONFIG_REFRESH);
        if stale {
            let config = config_rpc::load_config_with_timeout().await?;
            runtime.bind(&config).map_err(|e| format!("{e:#}"))?;
        }
        let visible = opt_bool(&p, "visible")?.ok_or("missing required param 'visible'")?;
        let errors: Vec<String> = match p.get("hotkey_errors") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|i| i.get("name").and_then(Value::as_str))
                .map(|n| n.chars().take(16).collect())
                .collect(),
            _ => Vec::new(),
        };
        Ok(ops::lease(runtime, visible, &errors))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_companion_function_has_a_schema_and_handler() {
        let all = all_controller_schemas();
        assert_eq!(all.len(), FUNCTIONS.len());
        for (s, name) in all.iter().zip(FUNCTIONS) {
            assert_eq!(s.namespace, "pet");
            assert_eq!(s.function, *name);
        }
        assert_eq!(all_registered_controllers().len(), FUNCTIONS.len());
        assert_eq!(schemas("bogus").function, "unknown");
    }

    #[test]
    fn update_schema_lists_every_patch_field() {
        let s = schemas("companion_update");
        let names: Vec<&str> = s.inputs.iter().map(|i| i.name).collect();
        let patch = serde_json::to_value(
            crate::neppy::pet::companion::settings::CompanionSettingsPatch::default(),
        )
        .unwrap();
        for key in patch.as_object().unwrap().keys() {
            assert!(
                names.contains(&key.as_str()),
                "schema misses patch field {key}"
            );
        }
        assert!(s.inputs.iter().all(|i| !i.required));
    }

    #[test]
    fn source_and_numbers_are_validated_by_name() {
        let mut p = Map::new();
        p.insert("source".into(), Value::from("elsewhere"));
        assert!(source(&p).unwrap_err().contains("'source'"));
        p.insert("limit".into(), Value::from(-3));
        assert!(opt_u64(&p, "limit").unwrap_err().contains("'limit'"));
        p.insert("all".into(), Value::from("yes"));
        assert!(opt_bool(&p, "all").unwrap_err().contains("'all'"));
    }
}
