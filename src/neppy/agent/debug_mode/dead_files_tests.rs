use super::*;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn changed(files: &[&str]) -> Vec<String> {
    files.iter().map(|s| s.to_string()).collect()
}

/// A small `app/src` tree: `Live` is rendered by `App`, `Dead` and `Orphan`
/// are imported by nothing (`Dead` only by its own test).
fn tree() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "app/src/main.tsx", "import App from './App';\n");
    write(
        r,
        "app/src/App.tsx",
        "import { Live } from './components/Live';\nimport { helper } from '@/lib/helper';\nimport type { T } from \"./types/t\";\n",
    );
    write(
        r,
        "app/src/components/Live.tsx",
        "export const Live = () => null;\n",
    );
    write(
        r,
        "app/src/components/Dead.tsx",
        "export const Dead = () => null;\n",
    );
    write(
        r,
        "app/src/components/Dead.test.tsx",
        "import { Dead } from './Dead';\n",
    );
    write(
        r,
        "app/src/components/Orphan.tsx",
        "export const Orphan = 1;\n",
    );
    write(r, "app/src/lib/helper.ts", "export const helper = 1;\n");
    write(r, "app/src/types/t.ts", "export type T = 1;\n");
    write(r, "app/src/types/ambient.d.ts", "declare const x: 1;\n");
    write(
        r,
        "app/src/components/index.ts",
        "export * from './Live';\n",
    );
    write(
        r,
        "app/src/lazy/Barrel.ts",
        "export * from '../components/Orphan';\n",
    );
    write(r, "app/src/components/Dyn.tsx", "export default 1;\n");
    write(
        r,
        "app/src/pages/Page.tsx",
        "const m = import('../components/Dyn');\n",
    );
    write(r, "app/src/pages/Mocked.tsx", "export default 2;\n");
    write(
        r,
        "app/src/pages/Mocked.test.tsx",
        "vi.mock('./Mocked', () => ({}));\n",
    );
    write(r, "src/lib.rs", "// rust\n");
    d
}

#[test]
fn warns_when_every_changed_source_file_is_unimported() {
    let d = tree();
    for files in [
        &["app/src/components/Dead.tsx"][..],
        &["app/src/components/Dead.tsx", "app/src/pages/Mocked.tsx"][..],
        // Rust and docs alongside do not matter.
        &["app/src/components/Dead.tsx", "src/lib.rs", "README.md"][..],
        // A test that imports it does not make it live.
        &[
            "app/src/components/Dead.tsx",
            "app/src/components/Dead.test.tsx",
        ][..],
    ] {
        assert_eq!(
            dead_file_warning(d.path(), &changed(files)).as_deref(),
            Some(WARNING),
            "{files:?}"
        );
    }
}

#[test]
fn no_warning_when_at_least_one_changed_file_is_imported() {
    let d = tree();
    for (name, files) in [
        ("relative import", &["app/src/components/Live.tsx"][..]),
        ("alias import", &["app/src/lib/helper.ts"][..]),
        (
            "import type with double quotes",
            &["app/src/types/t.ts"][..],
        ),
        (
            "barrel re-export counts",
            &["app/src/components/Orphan.tsx"][..],
        ),
        ("dynamic import counts", &["app/src/components/Dyn.tsx"][..]),
        (
            "one live among dead",
            &["app/src/components/Dead.tsx", "app/src/components/Live.tsx"][..],
        ),
    ] {
        assert_eq!(dead_file_warning(d.path(), &changed(files)), None, "{name}");
    }
}

#[test]
fn tests_declarations_index_entry_and_non_frontend_files_are_not_judged() {
    let d = tree();
    for (name, files) in [
        ("nothing changed", &[][..]),
        ("test only", &["app/src/components/Dead.test.tsx"][..]),
        ("d.ts only", &["app/src/types/ambient.d.ts"][..]),
        ("index only", &["app/src/components/index.ts"][..]),
        ("entry only", &["app/src/main.tsx"][..]),
        ("rust only", &["src/lib.rs"][..]),
        ("docs only", &["README.md", "app/src/index.css"][..]),
        ("outside app/src", &["app/vite.config.ts"][..]),
        ("deleted file", &["app/src/components/Gone.tsx"][..]),
    ] {
        assert_eq!(dead_file_warning(d.path(), &changed(files)), None, "{name}");
    }
}

#[test]
fn excluded_files_next_to_a_dead_one_do_not_hide_the_warning_or_cause_one() {
    let d = tree();
    // The dead file is judged; the test and index files are ignored.
    let files = changed(&[
        "app/src/components/Dead.tsx",
        "app/src/components/Dead.test.tsx",
        "app/src/components/index.ts",
        "app/src/types/ambient.d.ts",
    ]);
    assert_eq!(
        dead_file_warning(d.path(), &files).as_deref(),
        Some(WARNING)
    );
}

#[test]
fn spec_resolution_handles_extensions_queries_and_parent_dirs() {
    assert_eq!(
        resolve_spec("app/src/a/b.tsx", "../c/D.js").as_deref(),
        Some("app/src/c/D")
    );
    assert_eq!(
        resolve_spec("app/src/a/b.tsx", "./x.tsx?raw").as_deref(),
        Some("app/src/a/x")
    );
    assert_eq!(
        resolve_spec("app/src/a/b.tsx", "@/lib/y").as_deref(),
        Some("app/src/lib/y")
    );
    assert_eq!(resolve_spec("app/src/a/b.tsx", "react"), None);
    assert_eq!(resolve_spec("a.ts", "../../x"), None);
}

#[test]
fn the_warning_text_is_the_specified_one() {
    assert_eq!(
        WARNING,
        "Warning: none of the changed files is imported anywhere; check you edited the code that actually renders."
    );
}

mod report {
    use serde_json::json;

    use crate::neppy::agent::debug_mode::ops::{self, DebugCtx};
    use crate::neppy::agent::debug_mode::test_util::{repo_with_test_script, TEST_CHECK};
    use crate::neppy::agent::debug_mode::tools::{DebugReportTool, DebugRunCheckTool};
    use crate::neppy::agent::debug_mode::turn::{begin, resolve_root, with_turn};
    use crate::neppy::agent::debug_mode::types::TaskStatus;
    use crate::neppy::tools::traits::Tool;

    async fn report(files: &[(&str, &str)], status: &str) -> (String, TaskStatus, String) {
        let repo = repo_with_test_script();
        let ws = tempfile::tempdir().unwrap();
        let root = resolve_root(Some(repo.path().to_str().unwrap()))
            .await
            .unwrap();
        let turn = begin(ws.path(), root, "dead file test").await;
        let out = with_turn(turn.clone(), async {
            for (rel, body) in files {
                super::write(repo.path(), rel, body);
            }
            let r = DebugRunCheckTool
                .execute(json!({ "command": TEST_CHECK }))
                .await
                .unwrap();
            assert!(!r.is_error, "{}", r.text());
            DebugReportTool
                .execute(json!({"status": status, "summary": "changed the card"}))
                .await
                .unwrap()
                .text()
        })
        .await;
        let task = ops::task_get(&DebugCtx::new(ws.path()), turn.task_id.as_deref().unwrap())
            .await
            .unwrap()
            .value;
        (out, task.status, task.summary.unwrap_or_default())
    }

    #[tokio::test]
    async fn a_pass_over_only_unimported_files_carries_the_warning_but_stays_a_pass() {
        let (out, status, summary) = report(
            &[
                ("app/src/App.tsx", "export default 1;\n"),
                ("app/src/components/Old.tsx", "export const Old = 1;\n"),
            ],
            "pass",
        )
        .await;
        // `App.tsx` is imported by nobody in this fixture either.
        assert!(out.contains(super::WARNING), "{out}");
        assert!(summary.contains(super::WARNING), "{summary}");
        assert!(summary.starts_with("changed the card"));
        assert_eq!(status, TaskStatus::Pass, "a warning, not a downgrade");
    }

    #[tokio::test]
    async fn no_warning_when_a_changed_file_is_imported_or_the_report_is_not_a_pass() {
        let (out, status, summary) = report(
            &[
                ("app/src/App.tsx", "import { L } from './L';\n"),
                ("app/src/L.tsx", "export const L = 1;\n"),
            ],
            "pass",
        )
        .await;
        assert!(!out.contains("Warning:"), "{out}");
        assert_eq!(summary, "changed the card");
        assert_eq!(status, TaskStatus::Pass);

        let (out, status, _) =
            report(&[("app/src/components/Old.tsx", "export {};\n")], "partial").await;
        assert!(!out.contains("Warning:"), "{out}");
        assert_eq!(status, TaskStatus::Partial);
    }
}
