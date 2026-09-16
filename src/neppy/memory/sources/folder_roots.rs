//! Where a personal notes folder actually lives, and how to repair a folder
//! source that only ever recorded its name.
//!
//! The Add Memory Source picker used `<input type="file" webkitdirectory>` and
//! read `File.path` to recover an absolute path. `File.path` is a
//! Chromium/Electron extension and this app has run on Wry (WebKit on macOS)
//! since #5456, so it was always `undefined` and the code fell through to
//! `webkitRelativePath.split('/')[0]` — the folder's NAME. A source added that
//! way stored `"AI Memory Hub"`, which [`FolderReader`] resolves with a bare
//! `PathBuf::from` against the core's working directory, so every sync failed
//! with `folder does not exist`.
//!
//! The picker is fixed, but configs written before that still carry the broken
//! value, and nothing in the sync path could tell a name from a path.
//!
//! [`FolderReader`]: https://github.com/tinyhumansai/tinymemory

use std::path::{Path, PathBuf};

/// Obsidian's own iCloud container. A vault synced through Obsidian lives here.
const OBSIDIAN_ICLOUD: &str = "Library/Mobile Documents/iCloud~md~obsidian/Documents";
/// Apple's generic iCloud Drive container.
const ICLOUD_DRIVE: &str = "Library/Mobile Documents/com~apple~CloudDocs";

/// The marker that makes a directory an Obsidian vault rather than any folder.
const VAULT_MARKER: &str = ".obsidian";

/// Roots a notes folder is plausibly kept under, **most authoritative first**.
///
/// The order is the whole contract, not a convenience. The same vault is
/// routinely present in more than one of these — a copy under a general cloud
/// mount (Google Drive's `My Drive`, Dropbox) alongside the one Obsidian itself
/// syncs. Resolving to the duplicate would index a stale tree while looking
/// entirely correct, which is worse than not resolving at all, so the
/// application's own container is consulted before any general mount.
pub fn notes_roots() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    [
        OBSIDIAN_ICLOUD,
        "Documents",
        ICLOUD_DRIVE,
        "My Drive",
        "Dropbox",
        "",
    ]
    .iter()
    .map(|suffix| {
        if suffix.is_empty() {
            home.clone()
        } else {
            home.join(suffix)
        }
    })
    .collect()
}

/// Resolve a folder-source path that is not absolute against [`notes_roots`].
///
/// Returns `None` when nothing matches. That case must leave the stored value
/// exactly as it was: a path that does not exist produces a legible error,
/// whereas substituting a different wrong path produces a confusing one.
pub fn resolve_relative_folder_path(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() || Path::new(trimmed).is_absolute() {
        return None;
    }
    // `Path::join` does not normalise, so `../../Desktop` would happily resolve
    // to a directory nowhere near a notes root and be written back as if it had
    // been found there. A stored value with `..` in it is not something this
    // repair should interpret at all.
    if Path::new(trimmed)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    notes_roots()
        .into_iter()
        .map(|root| root.join(trimmed))
        .find(|candidate| candidate.is_dir())
}

/// A folder confident enough to PRE-FILL the Add Memory Source field with.
///
/// Only an unambiguous answer qualifies: exactly one Obsidian vault. The field
/// it fills is the value that gets saved, and the Add button enables as soon as
/// it holds an absolute path — so filling it with a *container* would put a
/// source indexing every vault inside it one click away. Several vaults (or
/// none) therefore yield `None` and the field stays empty; [`notes_browse_start`]
/// still opens the picker in the right place.
pub fn default_notes_folder() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let container = home.join(OBSIDIAN_ICLOUD);
    if !container.is_dir() {
        return None;
    }
    let mut vaults = obsidian_vaults_in(&container);
    if vaults.len() == 1 {
        return vaults.pop();
    }
    None
}

/// Where the native folder picker should OPEN.
///
/// A weaker claim than [`default_notes_folder`] and deliberately so: nothing is
/// saved from it, the user is about to choose, so a container holding several
/// vaults is exactly the right place to land.
pub fn notes_browse_start() -> Option<PathBuf> {
    notes_roots().into_iter().find(|root| root.is_dir())
}

/// Immediate subdirectories of `container` that carry the Obsidian marker.
fn obsidian_vaults_in(container: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(container) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join(VAULT_MARKER).exists())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_path_is_never_rewritten() {
        // Already correct: resolving it again could only break it.
        assert_eq!(resolve_relative_folder_path("/Users/someone/Notes"), None);
    }

    #[test]
    fn empty_and_whitespace_resolve_to_nothing() {
        assert_eq!(resolve_relative_folder_path(""), None);
        assert_eq!(resolve_relative_folder_path("   "), None);
    }

    #[test]
    fn a_name_matching_nothing_is_left_alone() {
        // The no-match case must stay `None` so the caller keeps the original
        // value and the user keeps a legible error.
        assert_eq!(
            resolve_relative_folder_path("a folder that does not exist anywhere 9f3c"),
            None
        );
    }

    #[test]
    fn obsidians_own_container_outranks_a_general_cloud_mount() {
        // The ordering that stops a duplicate copy winning. Asserted on the
        // list rather than the filesystem so it holds on any machine.
        let roots = notes_roots();
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let obsidian = roots.iter().position(|r| *r == home.join(OBSIDIAN_ICLOUD));
        let my_drive = roots.iter().position(|r| *r == home.join("My Drive"));
        if let (Some(obsidian), Some(my_drive)) = (obsidian, my_drive) {
            assert!(
                obsidian < my_drive,
                "Obsidian's container must be consulted before a general cloud mount"
            );
        }
    }

    #[test]
    fn a_parent_dir_escape_is_refused() {
        // `Path::join` does not normalise, so without this guard `../Desktop`
        // resolves to a real directory outside every notes root and is written
        // back as though it had been found in one.
        assert_eq!(resolve_relative_folder_path("../Desktop"), None);
        assert_eq!(
            resolve_relative_folder_path("Documents/../../Desktop"),
            None
        );
    }

    #[test]
    fn a_container_is_never_offered_as_a_prefill() {
        // The field this fills is submittable, so an ambiguous answer must be
        // no answer. With several vaults (or none) it declines; the picker
        // start point below is what still points at the right place.
        if let Some(folder) = default_notes_folder() {
            assert!(
                folder.join(VAULT_MARKER).exists(),
                "a prefill must be an actual vault, not the container holding several"
            );
        }
    }

    #[test]
    fn the_browse_start_is_allowed_to_be_a_container() {
        // Weaker claim, different job: nothing is saved from it.
        if let Some(start) = notes_browse_start() {
            assert!(start.is_dir());
        }
    }

    #[test]
    fn home_is_the_last_resort() {
        let roots = notes_roots();
        if let (Some(home), Some(last)) = (dirs::home_dir(), roots.last()) {
            assert_eq!(*last, home, "bare home must be tried after every container");
        }
    }
}
