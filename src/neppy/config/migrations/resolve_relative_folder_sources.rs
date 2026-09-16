//! 11 -> 12: give folder memory sources that stored a NAME an actual path.
//!
//! The Add Memory Source picker recovered its path from `File.path`, a
//! Chromium extension absent from this app's Wry/WebKit runtime, and silently
//! fell back to the chosen folder's name. A source added that way holds
//! `"AI Memory Hub"`, the folder reader resolves it against the core's working
//! directory, and every sync fails with `folder does not exist: AI Memory Hub`.
//!
//! The picker now uses a native dialog, so no NEW source can be written this
//! way. This repairs the ones already on disk, which the fix could not reach.
//!
//! **It never guesses.** A name is resolved only against the roots a notes
//! folder is actually kept under, and only when the result is a directory that
//! exists; anything unresolved keeps its original value so the user keeps a
//! legible error rather than a different, more confusing one.

use crate::neppy::config::Config;
use crate::neppy::memory::sources::folder_roots::resolve_relative_folder_path;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Sources whose path was rewritten to an absolute one.
    pub repaired: usize,
    /// Sources left alone because nothing plausible matched.
    pub unresolved: usize,
}

pub fn run(config: &mut Config) -> anyhow::Result<Stats> {
    let mut stats = Stats::default();

    for source in &mut config.memory_sources {
        // Only folder sources carry a filesystem path; a `url`-shaped source's
        // `path` field means something else entirely.
        if source.kind != crate::neppy::memory::sources::types::SourceKind::Folder {
            continue;
        }
        let Some(stored) = source.path.clone() else {
            continue;
        };
        match resolve_relative_folder_path(&stored) {
            Some(resolved) => {
                let resolved = resolved.display().to_string();
                log::info!(
                    "[migrations] resolve_relative_folder_sources: source {} path {:?} -> {:?}",
                    source.id,
                    stored,
                    resolved
                );
                source.path = Some(resolved);
                stats.repaired += 1;
            }
            None => {
                // Absolute paths land here too and are the common case, so this
                // counts only the ones that genuinely could not be resolved.
                if !std::path::Path::new(stored.trim()).is_absolute() && !stored.trim().is_empty() {
                    log::warn!(
                        "[migrations] resolve_relative_folder_sources: source {} keeps \
                         unresolvable path {:?} — no notes root contains it",
                        source.id,
                        stored
                    );
                    stats.unresolved += 1;
                }
            }
        }
    }

    Ok(stats)
}

#[cfg(test)]
#[path = "resolve_relative_folder_sources_tests.rs"]
mod tests;
