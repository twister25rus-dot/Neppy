//! A warning for the "edited a file nothing renders" mistake.
//!
//! The incident behind it: the model changed a component that was no longer
//! imported anywhere, validated green, and reported `pass` while the UI the user
//! was looking at never changed. When a task changed frontend source under
//! `app/src` and EVERY such file is unimported, [`dead_file_warning`] returns a
//! warning for the report. It never downgrades the status: a new module that a
//! later change will wire up is legitimate, so this only prompts a second look.
//!
//! Pure over the working tree: it reads `app/src` and decides by module path
//! (relative or `@/` alias, extension optional). Files that are tests, `.d.ts`
//! declarations, `index` / `main` entry points, or no longer on disk are not
//! judged. A test file importing a module does not make it live.

use std::path::{Component, Path};

use regex::Regex;
use walkdir::WalkDir;

/// Appended to the `debug_report` result and the task summary.
pub(super) const WARNING: &str = "Warning: none of the changed files is imported anywhere; \
check you edited the code that actually renders.";

const SRC_PREFIX: &str = "app/src/";
const CODE_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];
const MAX_SCANNED_FILES: usize = 20_000;

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map_or("", |(_, e)| e)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn is_test_path(path: &str) -> bool {
    let name = file_name(path);
    name.contains(".test.")
        || name.contains(".spec.")
        || name.contains(".stories.")
        || path.contains("/__tests__/")
        || path.contains("/__mocks__/")
        || path.starts_with("app/src/test/")
}

/// A changed file this check judges: ts/tsx under `app/src`, not a test, not a
/// `.d.ts`, not an `index` / `main` entry point.
fn is_judged_source(path: &str) -> bool {
    if !path.starts_with(SRC_PREFIX) || !matches!(ext(path), "ts" | "tsx") {
        return false;
    }
    let name = file_name(path);
    if name.ends_with(".d.ts") || is_test_path(path) {
        return false;
    }
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    !matches!(stem, "index" | "main")
}

/// `a/b/../c` -> `a/c`, with `.` dropped. `None` if it climbs above the root.
fn normalize(path: &Path) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn strip_module_ext(spec: &str) -> &str {
    let spec = spec.split(['?', '#']).next().unwrap_or(spec);
    for e in [".tsx", ".ts", ".jsx", ".js", ".mjs", ".cjs"] {
        if let Some(s) = spec.strip_suffix(e) {
            return s;
        }
    }
    spec
}

/// The repo-relative module path (no extension) that `spec` names when written
/// in `importer`, or `None` for a package or an unknown alias.
fn resolve_spec(importer: &str, spec: &str) -> Option<String> {
    let spec = strip_module_ext(spec);
    if let Some(rest) = spec.strip_prefix("@/") {
        return normalize(&Path::new("app/src").join(rest));
    }
    if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
        let dir = Path::new(importer).parent()?;
        return normalize(&dir.join(spec));
    }
    None
}

fn import_regex() -> Regex {
    Regex::new(
        r#"(?:\bfrom|\bimport|\brequire|\bmock|\bimportActual)\s*\(?\s*['"`]([^'"`\n]+)['"`]"#,
    )
    .expect("static import regex")
}

/// Every module path (no extension) that the non-test source files under
/// `app/src` import, as repo-relative strings.
fn imported_modules(root: &Path) -> std::collections::HashSet<String> {
    let re = import_regex();
    let mut out = std::collections::HashSet::new();
    let base = root.join("app/src");
    let walker = WalkDir::new(&base).into_iter().filter_entry(|e| {
        let n = e.file_name().to_string_lossy();
        n != "node_modules" && n != "dist"
    });
    for entry in walker.filter_map(Result::ok).take(MAX_SCANNED_FILES) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if !CODE_EXTS.contains(&ext(&rel)) || is_test_path(&rel) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        for cap in re.captures_iter(&text) {
            if let Some(m) = resolve_spec(&rel, &cap[1]) {
                // `import './dir'` may name `dir/index`.
                if m != rel.rsplit_once('.').map_or(rel.as_str(), |(s, _)| s) {
                    out.insert(m);
                }
            }
        }
    }
    out
}

fn module_path(file: &str) -> &str {
    file.rsplit_once('.').map_or(file, |(s, _)| s)
}

/// `Some(WARNING)` when the task changed frontend source under `app/src`, at
/// least one such file still exists, and none of them is imported by another
/// non-test file in `app/src`. `files` are repo-relative changed paths.
pub(super) fn dead_file_warning(root: &Path, files: &[String]) -> Option<String> {
    let candidates: Vec<&str> = files
        .iter()
        .map(String::as_str)
        .filter(|f| is_judged_source(f) && root.join(f).is_file())
        .collect();
    if candidates.is_empty() {
        log::debug!("[debug_mode] dead-file check: no judged source files changed");
        return None;
    }
    let imported = imported_modules(root);
    let live = candidates
        .iter()
        .filter(|f| imported.contains(module_path(f)))
        .count();
    log::debug!(
        "[debug_mode] dead-file check: candidates={} imported={live}",
        candidates.len()
    );
    (live == 0).then(|| WARNING.to_string())
}

#[cfg(test)]
#[path = "dead_files_tests.rs"]
mod tests;
