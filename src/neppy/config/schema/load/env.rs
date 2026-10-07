/// Read-only environment lookup used by [`crate::neppy::config::schema::Config::apply_env_overrides`].
/// The seam lets unit tests exercise the overlay without mutating the process
/// environment (which is racy under parallel tests and requires a shared
/// `TEST_ENV_LOCK`).
///
/// Production code uses [`ProcessEnv`], which delegates to `std::env`.
pub(crate) trait EnvLookup {
    /// Equivalent to `crate::neppy::util::env::var(key).ok()`.
    fn get(&self, key: &str) -> Option<String>;

    /// Equivalent to `crate::neppy::util::env::var_os(key).is_some()`. Used to distinguish
    /// "variable not present" from "variable set to empty" where it matters
    /// (see `NEPPY_CONTEXT_TOOL_RESULT_BUDGET_BYTES` below).
    fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Looks up the first non-`None` value across `keys`, preserving the
    /// precedence used by the manual `or_else` chains throughout this
    /// module (e.g. `NEPPY_FOO` wins over the bare `FOO` alias).
    fn get_any(&self, keys: &[&str]) -> Option<String> {
        keys.iter().find_map(|k| self.get(k))
    }
}

/// Default [`EnvLookup`] implementation backed by `std::env`.
pub(crate) struct ProcessEnv;

impl EnvLookup for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        crate::neppy::util::env::var(key).ok()
    }

    fn contains(&self, key: &str) -> bool {
        crate::neppy::util::env::var_os(key).is_some()
    }
}

/// Process env lookup that preserves every override except
/// `NEPPY_WORKSPACE`.
pub(crate) struct ProcessEnvWithoutWorkspace;

impl EnvLookup for ProcessEnvWithoutWorkspace {
    fn get(&self, key: &str) -> Option<String> {
        if key == "NEPPY_WORKSPACE" {
            None
        } else {
            ProcessEnv.get(key)
        }
    }

    fn contains(&self, key: &str) -> bool {
        if key == "NEPPY_WORKSPACE" {
            false
        } else {
            ProcessEnv.contains(key)
        }
    }
}

/// Parse a boolean env-var value. Accepts the usual truthy/falsy tokens
/// (`1/true/yes/on` and `0/false/no/off`, case-insensitive). Returns `None`
/// on unrecognised values and logs a warning so silent mis-spellings don't
/// invisibly leave the config unchanged.
pub(super) fn parse_env_bool(name: &str, raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => {
            tracing::warn!(
                env = %name,
                value = %raw,
                "invalid boolean env override ignored; expected 1/true/yes/on or 0/false/no/off"
            );
            None
        }
    }
}
