//! Debug Mode write guard: inside a Debug turn, files the debug runtime must
//! not be able to rewrite (the recovery tool — see
//! `agent::debug_mode::selfmod::PROTECTED_PATTERNS`) are refused for writes.
//! Outside a Debug turn this is a no-op, so normal behaviour is unchanged.

use std::path::Path;

use super::types::{SecurityPolicy, POLICY_BLOCKED_MARKER};
use crate::neppy::agent::{debug_mode, turn_workspace};

impl SecurityPolicy {
    /// Refuses a WRITE to a Debug-protected path. `resolved` is the canonical
    /// target; it is made repo-relative against the turn's project root.
    /// Reads never call this.
    pub fn check_debug_protected_write(&self, resolved: &Path) -> Result<(), String> {
        if debug_mode::turn::current().is_none() {
            return Ok(());
        }
        let Some(root) = turn_workspace::current() else {
            return Ok(());
        };
        let canonical_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let rel = resolved
            .strip_prefix(&canonical_root)
            .or_else(|_| resolved.strip_prefix(&root));
        let Ok(rel) = rel else {
            return Ok(());
        };
        let rel = rel.to_string_lossy();
        if debug_mode::selfmod::is_protected_path(&rel) {
            log::warn!("[security:policy] debug turn write refused to protected path '{rel}'");
            return Err(format!(
                "{POLICY_BLOCKED_MARKER} '{rel}' is protected in Debug mode (it is the recovery \
                 path and must stay intact). Do not retry; ask the user to change it by hand."
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "debug_guard_tests.rs"]
mod tests;
