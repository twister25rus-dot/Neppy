//! Load / update helpers for `[debug_mode]` in the main config file.
//!
//! Updates go through the same persistence path as every other
//! `update_*_settings` controller: load the config, mutate, `Config::save`.
//! Nothing else is touched, and the audit log records field *names* only.

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use crate::neppy::config::rpc as config_rpc;
use crate::neppy::config::schema::debug_mode::{
    DebugModeConfig, MAX_REPAIR_ITERATIONS, MIN_REPAIR_ITERATIONS,
};

use super::ops::{audit, done, DebugCtx, RpcResult};
use super::policy;

/// What `debug_mode_settings_get` returns (snake_case, same fields as config).
pub type DebugModeSettings = DebugModeConfig;

/// Serialises read-modify-write of the config file.
static UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// Partial update: an absent field is left alone; `project_root: null` clears it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "double_option")]
    pub project_root: Option<Option<String>>,
    pub auto_checkpoint: Option<bool>,
    pub auto_repair: Option<bool>,
    pub max_repair_iterations: Option<i64>,
    pub run_tests_after_changes: Option<bool>,
    pub run_build_after_changes: Option<bool>,
    pub allow_dependency_install: Option<bool>,
    pub allow_external_filesystem: Option<bool>,
    pub external_paths: Option<Vec<String>>,
    pub allow_system_commands: Option<bool>,
    pub allow_git_commit: Option<bool>,
    pub allow_git_push: Option<bool>,
    pub dangerous_commands_require_confirmation: Option<bool>,
}

impl SettingsPatch {
    /// Names of the fields this patch sets (no values), for the audit log.
    pub fn field_names(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        macro_rules! f {
            ($($n:ident),*) => {$( if self.$n.is_some() { v.push(stringify!($n)); } )*};
        }
        f!(
            enabled,
            project_root,
            auto_checkpoint,
            auto_repair,
            max_repair_iterations,
            run_tests_after_changes,
            run_build_after_changes,
            allow_dependency_install,
            allow_external_filesystem,
            external_paths,
            allow_system_commands,
            allow_git_commit,
            allow_git_push,
            dangerous_commands_require_confirmation
        );
        v
    }
}

/// The saved settings; an unreadable config is an error (callers must not
/// guess permissions).
pub async fn load() -> Result<DebugModeConfig, String> {
    Ok(config_rpc::load_config_with_timeout().await?.debug_mode)
}

/// Validates `patch` and applies it to `current`. Pure apart from the
/// filesystem checks on `project_root` / `external_paths`.
pub async fn apply_patch(
    current: &DebugModeConfig,
    patch: SettingsPatch,
) -> Result<DebugModeConfig, String> {
    let mut next = current.clone();
    if let Some(n) = patch.max_repair_iterations {
        let (lo, hi) = (MIN_REPAIR_ITERATIONS as i64, MAX_REPAIR_ITERATIONS as i64);
        if !(lo..=hi).contains(&n) {
            return Err(format!(
                "max_repair_iterations must be between {lo} and {hi} (got {n})"
            ));
        }
        next.max_repair_iterations = n as u32;
    }
    if let Some(root) = patch.project_root {
        next.project_root = match root.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            None => None,
            Some(r) => Some(
                super::git::resolve_project_root(Some(&r))
                    .await
                    .map_err(|e| format!("invalid project_root: {e}"))?
                    .display()
                    .to_string(),
            ),
        };
    }
    if let Some(paths) = patch.external_paths {
        let mut out: Vec<String> = Vec::new();
        for raw in &paths {
            let canon = policy::validate_external_path(raw)?.display().to_string();
            if !out.contains(&canon) {
                out.push(canon);
            }
        }
        next.external_paths = out;
    }
    macro_rules! set {
        ($($n:ident),*) => {$( if let Some(v) = patch.$n { next.$n = v; } )*};
    }
    set!(
        enabled,
        auto_checkpoint,
        auto_repair,
        run_tests_after_changes,
        run_build_after_changes,
        allow_dependency_install,
        allow_external_filesystem,
        allow_system_commands,
        allow_git_commit,
        allow_git_push,
        dangerous_commands_require_confirmation
    );
    Ok(next)
}

pub async fn get(ctx: &DebugCtx) -> RpcResult<DebugModeSettings> {
    let r = load().await;
    audit(ctx, "settings_get", "-", &r);
    r.and_then(done)
}

pub async fn update(ctx: &DebugCtx, patch: Map<String, Value>) -> RpcResult<DebugModeSettings> {
    let mut target = "-".to_string();
    let r = async {
        let patch: SettingsPatch = serde_json::from_value(Value::Object(patch))
            .map_err(|e| format!("invalid settings patch: {e}"))?;
        target = patch.field_names().join(",");
        let _g = UPDATE_LOCK.lock().await;
        let mut config = config_rpc::load_config_with_timeout().await?;
        let next = apply_patch(&config.debug_mode, patch).await?;
        config.debug_mode = next.clone();
        config.save().await.map_err(|e| e.to_string())?;
        log::info!("[debug_mode] settings updated fields={target}");
        Ok(next)
    }
    .await;
    audit(ctx, "settings_update", &target, &r);
    r.and_then(done)
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
