/**
 * Validation for a local-folder source's path.
 *
 * Shared by the add dialog and the per-source settings panel so the rule that
 * decides whether a path can be saved is written once.
 *
 * The rule is "absolute", and it is load-bearing rather than fussy. The core's
 * folder reader does `PathBuf::from(path)` with no expansion and no resolution
 * against a base, so anything relative is resolved against whatever the core's
 * working directory happens to be and almost never exists. A bare folder NAME
 * is the case that actually shipped: the old `webkitdirectory` picker stored
 * `"AI Memory Hub"` instead of its path, which is a perfectly valid relative
 * path, so nothing rejected it and every sync failed with `folder does not
 * exist` long after the mistake was made.
 */

/** Windows drive-letter root, e.g. `C:\Users` or `C:/Users`. */
const WINDOWS_ABSOLUTE = /^[A-Za-z]:[\\/]/;
/** UNC share, e.g. `\\server\share`. */
const UNC_ABSOLUTE = /^\\\\[^\\]/;

/**
 * Whether `path` is absolute on any platform this app ships to.
 *
 * Deliberately accepts all three shapes regardless of the host: settings sync
 * between machines, and a Windows path pasted on macOS should read as "a path
 * this machine cannot reach" rather than "not a path".
 *
 * `~` is NOT accepted. The core does no tilde expansion, so `~/Notes` would be
 * taken literally, look absolute to a reader, and fail exactly like the bare
 * name did.
 */
export function isAbsoluteFolderPath(path: string): boolean {
  const trimmed = path.trim();
  if (trimmed.length === 0) return false;
  return trimmed.startsWith('/') || WINDOWS_ABSOLUTE.test(trimmed) || UNC_ABSOLUTE.test(trimmed);
}
