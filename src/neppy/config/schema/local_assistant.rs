//! `[local_assistant]` — budgets and bounds for the on-device assistant.
//!
//! Pure data. The task controller, project index and checkpoint store that
//! consume it live under `inference::local::assistant`. Every field has a
//! default, so an existing `config.toml` without this table deserializes
//! unchanged and needs no migration.
//!
//! The limits are deliberately explicit numbers rather than "whatever the
//! model allows": the context limit matches the server's `max_kv_size`, and
//! the per-step and per-task token budgets bound how long one task can hold
//! the single inference slot.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LocalAssistantConfig {
    /// Master switch. `false` finishes or checkpoints the current step and
    /// accepts no new work.
    pub enabled: bool,
    /// Model id to run. Empty means the primary `[[mlx.server]]` block's model.
    pub model: String,
    /// Context window, tokens. Must not exceed the server's `max_kv_size`.
    pub context_limit_tokens: u32,
    /// Prompt budget per step, tokens.
    pub prompt_budget_tokens: u32,
    /// `max_tokens` sent with each step.
    pub step_max_tokens: u32,
    /// Steps per task before it ends as `budget_exhausted`.
    pub max_steps: u32,
    /// Completion tokens per task before it ends as `budget_exhausted`.
    pub task_max_completion_tokens: u32,
    /// Characters of retrieved snippets per step.
    pub snippet_budget_chars: usize,
    /// Snippets per step.
    pub max_snippets: usize,
    /// Files per index transaction.
    pub index_batch_files: usize,
    /// Files larger than this are not indexed.
    pub index_max_file_bytes: u64,
    /// Files indexed per project.
    pub index_max_files: usize,
    /// Bytes indexed per project.
    pub index_max_bytes: u64,
    /// Extra glob patterns excluded from the index, relative to the root.
    pub exclude_globs: Vec<String>,
    /// Tasks allowed to wait in the queue.
    pub task_queue_cap: usize,
    /// Finished tasks retained.
    pub keep_tasks: usize,
    /// Days finished tasks are retained.
    pub keep_days: u32,
    /// Seconds a test command may run before it is killed.
    pub test_timeout_secs: u64,
    /// Let the assistant edit files that run code on their own or configure the
    /// tooling that does. Off by default, so a model reading a hostile
    /// repository cannot plant something the user's next build, commit or shell
    /// runs. Refused unless this is `true`:
    ///
    /// - git hooks and editor, CI and package-manager configuration:
    ///   `.husky/`, `.githooks/`, `.vscode/`, `.idea/`, `.devcontainer/`,
    ///   `.circleci/`, `.github/` (actions and workflows), `.yarn/`,
    ///   `.cargo/config*`;
    /// - build and package manifests and scripts: `Cargo.toml` (its `build =`
    ///   key can name any file as a build script), `build.rs`, `package.json`,
    ///   `.npmrc`, `.yarnrc`, `.yarnrc.yml`, `.pnpmfile.cjs`, `Makefile`,
    ///   `GNUmakefile`, `*.mk`, `justfile`, `Rakefile`, `Gemfile`,
    ///   `pyproject.toml`, `setup.py`, `setup.cfg`, `conftest.py`, `tox.ini`,
    ///   `noxfile.py`, `CMakeLists.txt`, `build.gradle(.kts)`,
    ///   `settings.gradle(.kts)`, `pom.xml`;
    /// - environment and shell files: `.envrc`, `.mise.toml`, `.tool-versions`,
    ///   `.bashrc`, `.bash_profile`, `.profile`, `.zshrc`, `.zshenv` and their
    ///   siblings, `.gitconfig`, `.gitmodules`, `.pre-commit-config.yaml`,
    ///   `.gitlab-ci.yml`;
    /// - bare-repository plants: any file named `HEAD` or `config`, and any
    ///   directory whose name ends in `.git`;
    /// - any path with a non-ASCII component. Filesystems such as APFS fold
    ///   names (`package.jſon` opens `package.json`), so a spelling check alone
    ///   can be walked around; the check also runs on the name each component
    ///   has on disk.
    pub allow_sensitive_paths: bool,
}

impl Default for LocalAssistantConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: String::new(),
            context_limit_tokens: 16_384,
            prompt_budget_tokens: 11_000,
            step_max_tokens: 1_536,
            max_steps: 8,
            task_max_completion_tokens: 10_000,
            snippet_budget_chars: 24_000,
            max_snippets: 12,
            index_batch_files: 200,
            index_max_file_bytes: 524_288,
            index_max_files: 60_000,
            index_max_bytes: 268_435_456,
            exclude_globs: vec!["vendor/*/vendor/**".to_string()],
            task_queue_cap: 16,
            keep_tasks: 50,
            keep_days: 30,
            test_timeout_secs: 600,
            allow_sensitive_paths: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_table_takes_every_default() {
        let parsed: LocalAssistantConfig = toml::from_str("").expect("empty table parses");
        assert_eq!(parsed, LocalAssistantConfig::default());
        assert!(parsed.enabled);
        assert_eq!(parsed.exclude_globs, vec!["vendor/*/vendor/**".to_string()]);
    }

    #[test]
    fn the_default_budgets_fit_the_context_limit() {
        let config = LocalAssistantConfig::default();
        assert!(
            config.prompt_budget_tokens + config.step_max_tokens < config.context_limit_tokens,
            "a step's prompt plus completion must fit the context window"
        );
    }

    #[test]
    fn sensitive_paths_are_off_unless_opted_in() {
        assert!(!LocalAssistantConfig::default().allow_sensitive_paths);
        let parsed: LocalAssistantConfig =
            toml::from_str("allow_sensitive_paths = true\n").expect("parses");
        assert!(parsed.allow_sensitive_paths);
        assert_eq!(parsed.max_steps, LocalAssistantConfig::default().max_steps);
    }

    #[test]
    fn a_partial_table_keeps_the_other_defaults() {
        let parsed: LocalAssistantConfig =
            toml::from_str("max_steps = 3\nexclude_globs = []\n").expect("partial table parses");
        assert_eq!(parsed.max_steps, 3);
        assert!(parsed.exclude_globs.is_empty());
        assert_eq!(parsed.step_max_tokens, 1_536);
    }
}
