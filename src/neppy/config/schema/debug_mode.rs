//! `[debug_mode]` — in-app self-development environment.
//!
//! Every field carries a serde default so a `config.toml` written before this
//! block existed loads unchanged. The dangerous capabilities (dependency
//! install, external filesystem, system commands, git push) default to off.
//! Enforcement lives in `neppy::agent::debug_mode::policy`; this type is data.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lowest accepted `max_repair_iterations`.
pub const MIN_REPAIR_ITERATIONS: u32 = 1;
/// Highest accepted `max_repair_iterations`.
pub const MAX_REPAIR_ITERATIONS: u32 = 20;

fn d_true() -> bool {
    true
}
fn d_false() -> bool {
    false
}
fn d_iterations() -> u32 {
    5
}
fn d_turn_timeout() -> u64 {
    DEFAULT_TURN_TIMEOUT_SECS
}

/// Default wall-clock budget of one Debug-mode turn, in seconds (60 minutes).
/// Other agents get ~10 minutes; repository work (a cold `cargo build`, a full
/// test run, a candidate validation) routinely takes longer than that.
pub const DEFAULT_TURN_TIMEOUT_SECS: u64 = 3600;

/// Configuration for Debug Mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DebugModeConfig {
    /// Master switch. Off refuses every Debug-mode chat turn.
    #[serde(default = "d_true")]
    pub enabled: bool,
    /// Git work-tree root the agent edits. `None` falls back to the build-time
    /// source dir. `NEPPY_DEBUG_PROJECT_ROOT` and explicit params win over it.
    #[serde(default)]
    pub project_root: Option<String>,
    /// Take a checkpoint before each Debug-mode turn.
    #[serde(default = "d_true")]
    pub auto_checkpoint: bool,
    /// Let the agent iterate on failing checks by itself.
    #[serde(default = "d_true")]
    pub auto_repair: bool,
    /// Repair attempts per task (clamped to 1..=20 when read).
    #[serde(default = "d_iterations")]
    pub max_repair_iterations: u32,
    #[serde(default = "d_true")]
    pub run_tests_after_changes: bool,
    #[serde(default = "d_true")]
    pub run_build_after_changes: bool,
    /// Allow `npm add`, `cargo add`, `pip install <pkg>` and friends.
    #[serde(default = "d_false")]
    pub allow_dependency_install: bool,
    /// Allow the agent to touch `external_paths` in addition to the project.
    #[serde(default = "d_false")]
    pub allow_external_filesystem: bool,
    /// Absolute directories granted when `allow_external_filesystem` is on.
    #[serde(default)]
    pub external_paths: Vec<String>,
    /// Allow `sudo`, `launchctl`, `chown`, ... inside a Debug turn.
    #[serde(default = "d_false")]
    pub allow_system_commands: bool,
    #[serde(default = "d_true")]
    pub allow_git_commit: bool,
    #[serde(default = "d_false")]
    pub allow_git_push: bool,
    /// Ask before `rm -rf`, `git reset --hard` and similar.
    #[serde(default = "d_true")]
    pub dangerous_commands_require_confirmation: bool,
    /// Wall-clock budget of one Debug-mode turn, in seconds. `0` removes the
    /// ceiling. Applies to Debug turns only; every other agent keeps the
    /// default 10-minute turn budget.
    #[serde(default = "d_turn_timeout")]
    pub turn_timeout_secs: u64,
}

impl Default for DebugModeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            project_root: None,
            auto_checkpoint: true,
            auto_repair: true,
            max_repair_iterations: d_iterations(),
            run_tests_after_changes: true,
            run_build_after_changes: true,
            allow_dependency_install: false,
            allow_external_filesystem: false,
            external_paths: Vec::new(),
            allow_system_commands: false,
            allow_git_commit: true,
            allow_git_push: false,
            dangerous_commands_require_confirmation: true,
            turn_timeout_secs: DEFAULT_TURN_TIMEOUT_SECS,
        }
    }
}

impl DebugModeConfig {
    /// `max_repair_iterations` clamped to the supported range.
    pub fn repair_iterations(&self) -> u32 {
        self.max_repair_iterations
            .clamp(MIN_REPAIR_ITERATIONS, MAX_REPAIR_ITERATIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_spec() {
        let c = DebugModeConfig::default();
        assert!(c.enabled && c.auto_checkpoint && c.auto_repair);
        assert!(c.run_tests_after_changes && c.run_build_after_changes);
        assert_eq!(c.max_repair_iterations, 5);
        assert!(c.project_root.is_none() && c.external_paths.is_empty());
        assert!(!c.allow_dependency_install && !c.allow_external_filesystem);
        assert!(!c.allow_system_commands && !c.allow_git_push);
        assert!(c.allow_git_commit && c.dangerous_commands_require_confirmation);
        assert_eq!(c.turn_timeout_secs, 3600, "a Debug turn gets an hour");
    }

    #[test]
    fn turn_timeout_is_configurable_and_defaults_when_absent() {
        let c: DebugModeConfig = toml::from_str("turn_timeout_secs = 7200").unwrap();
        assert_eq!(c.turn_timeout_secs, 7200);
        let c: DebugModeConfig = toml::from_str("enabled = true").unwrap();
        assert_eq!(c.turn_timeout_secs, DEFAULT_TURN_TIMEOUT_SECS);
    }

    #[test]
    fn empty_table_and_partial_table_fill_defaults() {
        let c: DebugModeConfig = toml::from_str("").unwrap();
        assert_eq!(c, DebugModeConfig::default());
        let c: DebugModeConfig = toml::from_str("allow_git_push = true\nenabled = false").unwrap();
        assert!(c.allow_git_push && !c.enabled && c.allow_git_commit);
    }

    #[test]
    fn toml_round_trip_and_clamp() {
        let mut c = DebugModeConfig {
            project_root: Some("/x".into()),
            external_paths: vec!["/a".into()],
            max_repair_iterations: 99,
            ..Default::default()
        };
        let s = toml::to_string(&c).unwrap();
        assert_eq!(toml::from_str::<DebugModeConfig>(&s).unwrap(), c);
        assert_eq!(c.repair_iterations(), 20);
        c.max_repair_iterations = 0;
        assert_eq!(c.repair_iterations(), 1);
    }

    #[test]
    fn root_config_without_section_loads() {
        let mut v = toml::Value::try_from(crate::neppy::config::Config::default()).unwrap();
        assert!(v.as_table_mut().unwrap().remove("debug_mode").is_some());
        let s = toml::to_string(&v).unwrap();
        let c: crate::neppy::config::Config = toml::from_str(&s).unwrap();
        assert_eq!(c.debug_mode, DebugModeConfig::default());
    }
}
