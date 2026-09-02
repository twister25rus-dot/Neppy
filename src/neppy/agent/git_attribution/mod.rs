//! Git commit attribution for changes made by the Neppy agent.
//!
//! Agent shell commands inherit a temporary `prepare-commit-msg` hook which
//! appends Neppy's co-author trailer. The hook directory contains shims for
//! every client-side hook, so a repository's validation hooks continue to run.

mod hook;

pub use hook::hook_env;

#[cfg(test)]
mod tests;
