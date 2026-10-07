//! Environment-variable reads with a legacy-name fallback.
//!
//! The product's variables are spelled `NEPPY_*` (and `VITE_NEPPY_*` for the
//! build-time frontend ones). Before the rebrand they were `OPENHUMAN_*`, and
//! users still have those names in shells, launchd plists, `.env` files and
//! docker setups. Every read of one of ours goes through [`var`] / [`var_os`],
//! which take the canonical name, look it up, and fall back to the legacy
//! spelling when the canonical one is absent. The canonical name always wins
//! when both are set.
//!
//! Names that do not start with a managed prefix (`PATH`, `HOME`, `RUST_LOG`,
//! third-party variables, …) pass straight through, so a call site never has to
//! decide whether a name is "ours". A name passed in the *legacy* spelling is
//! normalised to the canonical one first, so a computed name cannot bypass the
//! precedence rule.
//!
//! Tests that need a variable to be absent must use [`remove_var`], which clears
//! both spellings; removing only `NEPPY_X` would let a stray `OPENHUMAN_X` in a
//! developer's shell leak through the fallback.
//!
//! Logging: the first time a legacy spelling is the one that supplies a value,
//! one `[env]` debug line names the variable (never its value).

use std::collections::HashSet;
use std::env::VarError;
use std::ffi::OsString;
use std::sync::{Mutex, OnceLock};

/// Canonical prefix of product environment variables.
pub const CANONICAL_PREFIX: &str = "NEPPY_";
/// Pre-rebrand prefix, still honoured as a fallback.
pub const LEGACY_PREFIX: &str = "OPENHUMAN_";
/// Canonical prefix of build-time frontend variables.
pub const CANONICAL_VITE_PREFIX: &str = "VITE_NEPPY_";
/// Pre-rebrand prefix of build-time frontend variables.
pub const LEGACY_VITE_PREFIX: &str = "VITE_OPENHUMAN_";

/// Returns `(canonical, legacy)` spellings of `name`, or `None` when `name` is
/// not one of ours. Accepts either spelling as input.
fn spellings(name: &str) -> Option<(String, String)> {
    for (canonical, legacy) in [
        (CANONICAL_VITE_PREFIX, LEGACY_VITE_PREFIX),
        (CANONICAL_PREFIX, LEGACY_PREFIX),
    ] {
        if let Some(rest) = name
            .strip_prefix(canonical)
            .or_else(|| name.strip_prefix(legacy))
        {
            return Some((format!("{canonical}{rest}"), format!("{legacy}{rest}")));
        }
    }
    None
}

/// The legacy (`OPENHUMAN_*`) spelling of a canonical name, if it has one.
pub fn legacy_name(name: &str) -> Option<String> {
    spellings(name).map(|(_, legacy)| legacy)
}

fn note_legacy_use(legacy: &str, canonical: &str) {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
    let first = match seen.lock() {
        Ok(mut set) => set.insert(legacy.to_string()),
        Err(_) => false,
    };
    if first {
        tracing::debug!(
            "[env] using legacy variable {legacy}; prefer {canonical} (value not logged)"
        );
    }
}

/// Drop-in for [`std::env::var`] that falls back to the legacy spelling.
pub fn var(name: &str) -> Result<String, VarError> {
    let Some((canonical, legacy)) = spellings(name) else {
        return std::env::var(name);
    };
    match std::env::var(&canonical) {
        Err(VarError::NotPresent) => {
            let value = std::env::var(&legacy);
            if value.is_ok() {
                note_legacy_use(&legacy, &canonical);
            }
            value
        }
        other => other,
    }
}

/// Drop-in for [`std::env::var_os`] that falls back to the legacy spelling.
pub fn var_os(name: &str) -> Option<OsString> {
    let Some((canonical, legacy)) = spellings(name) else {
        return std::env::var_os(name);
    };
    if let Some(value) = std::env::var_os(&canonical) {
        return Some(value);
    }
    let value = std::env::var_os(&legacy);
    if value.is_some() {
        note_legacy_use(&legacy, &canonical);
    }
    value
}

/// Removes `name` from the process environment, **both spellings**.
///
/// For tests and for code that must guarantee a variable is unset. Names that
/// are not ours are removed as-is.
pub fn remove_var(name: &str) {
    match spellings(name) {
        Some((canonical, legacy)) => {
            std::env::remove_var(canonical);
            std::env::remove_var(legacy);
        }
        None => std::env::remove_var(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each test uses its own variable names so parallel tests never race.

    #[test]
    fn canonical_wins_over_legacy() {
        std::env::set_var("NEPPY_ENVTEST_BOTH", "new");
        std::env::set_var("OPENHUMAN_ENVTEST_BOTH", "old");
        assert_eq!(var("NEPPY_ENVTEST_BOTH").as_deref(), Ok("new"));
        assert_eq!(var_os("NEPPY_ENVTEST_BOTH"), Some(OsString::from("new")));
        remove_var("NEPPY_ENVTEST_BOTH");
    }

    #[test]
    fn legacy_alone_is_honoured() {
        std::env::set_var("OPENHUMAN_ENVTEST_LEGACY", "old");
        assert_eq!(var("NEPPY_ENVTEST_LEGACY").as_deref(), Ok("old"));
        assert_eq!(var_os("NEPPY_ENVTEST_LEGACY"), Some(OsString::from("old")));
        remove_var("NEPPY_ENVTEST_LEGACY");
    }

    #[test]
    fn neither_is_not_present() {
        remove_var("NEPPY_ENVTEST_NONE");
        assert_eq!(var("NEPPY_ENVTEST_NONE"), Err(VarError::NotPresent));
        assert_eq!(var_os("NEPPY_ENVTEST_NONE"), None);
    }

    #[test]
    fn legacy_spelling_input_is_normalised() {
        std::env::set_var("NEPPY_ENVTEST_NORM", "new");
        std::env::set_var("OPENHUMAN_ENVTEST_NORM", "old");
        assert_eq!(var("OPENHUMAN_ENVTEST_NORM").as_deref(), Ok("new"));
        remove_var("OPENHUMAN_ENVTEST_NORM");
    }

    #[test]
    fn vite_names_fall_back_too() {
        std::env::set_var("VITE_OPENHUMAN_ENVTEST_V", "old");
        assert_eq!(var("VITE_NEPPY_ENVTEST_V").as_deref(), Ok("old"));
        std::env::set_var("VITE_NEPPY_ENVTEST_V", "new");
        assert_eq!(var("VITE_NEPPY_ENVTEST_V").as_deref(), Ok("new"));
        remove_var("VITE_NEPPY_ENVTEST_V");
        assert_eq!(var("VITE_NEPPY_ENVTEST_V"), Err(VarError::NotPresent));
    }

    #[test]
    fn remove_var_clears_both_spellings() {
        std::env::set_var("NEPPY_ENVTEST_RM", "a");
        std::env::set_var("OPENHUMAN_ENVTEST_RM", "b");
        remove_var("NEPPY_ENVTEST_RM");
        assert!(std::env::var_os("NEPPY_ENVTEST_RM").is_none());
        assert!(std::env::var_os("OPENHUMAN_ENVTEST_RM").is_none());
        assert_eq!(var("NEPPY_ENVTEST_RM"), Err(VarError::NotPresent));
    }

    #[test]
    fn foreign_names_pass_through() {
        std::env::set_var("ENVTEST_FOREIGN_X", "v");
        assert_eq!(var("ENVTEST_FOREIGN_X").as_deref(), Ok("v"));
        assert_eq!(legacy_name("ENVTEST_FOREIGN_X"), None);
        remove_var("ENVTEST_FOREIGN_X");
        assert_eq!(var("ENVTEST_FOREIGN_X"), Err(VarError::NotPresent));
    }

    #[test]
    fn legacy_name_maps_prefixes() {
        assert_eq!(
            legacy_name("NEPPY_WORKSPACE").as_deref(),
            Some("OPENHUMAN_WORKSPACE")
        );
        assert_eq!(
            legacy_name("VITE_NEPPY_APP_ENV").as_deref(),
            Some("VITE_OPENHUMAN_APP_ENV")
        );
    }
}
