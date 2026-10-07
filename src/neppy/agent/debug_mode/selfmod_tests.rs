use super::*;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn directory_globs_match_everything_below_but_not_siblings() {
    assert!(is_critical_path("src/neppy/agent/debug_mode/tools.rs"));
    assert!(is_critical_path("src/neppy/agent/debug_mode/sub/deep.rs"));
    assert!(is_critical_path("src/core/jsonrpc.rs"));
    assert!(is_critical_path("app/src-tauri/src/lib.rs"));
    assert!(!is_critical_path("src/neppy/agent/debug_mode_other.rs"));
    assert!(!is_critical_path("src/corex/foo.rs"));
    assert!(!is_critical_path("src/neppy/agent/harness.rs"));
}

#[test]
fn exact_and_prefix_patterns() {
    for p in [
        "src/main.rs",
        "src/lib.rs",
        "Cargo.toml",
        "Cargo.lock",
        "app/src/pages/DebugPage.tsx",
        "scripts/release-neppy.sh",
        "scripts/release/merge.sh",
        "scripts/neppy-recover.sh",
        "updater/latest.json",
        "src/neppy/agent/turn_workspace.rs",
    ] {
        assert!(is_critical_path(p), "{p}");
    }
    for p in [
        "app/Cargo.toml",
        "src/neppy/agent/turn_workspace.rs.bak",
        "scripts/build.sh",
        "app/src/pages/Home.tsx",
        "README.md",
    ] {
        assert!(!is_critical_path(p), "{p}");
    }
}

#[test]
fn matching_ignores_case_dot_prefix_and_backslashes() {
    assert!(is_critical_path("./cargo.TOML"));
    assert!(is_critical_path("SRC\\core\\auth.rs"));
    assert!(is_critical_path("/src/main.rs"));
}

#[test]
fn parent_traversal_is_never_treated_as_safe() {
    assert!(is_critical_path("docs/../../etc/passwd"));
    assert!(is_protected_path("../scripts/x"));
}

#[test]
fn assess_lists_sorted_unique_critical_files_only() {
    let a = assess(&s(&[
        "src/core/b.rs",
        "README.md",
        "Cargo.toml",
        "src/core/b.rs",
    ]));
    assert!(a.critical);
    assert_eq!(a.critical_files, s(&["Cargo.toml", "src/core/b.rs"]));
    let none = assess(&s(&["README.md", "app/src/pages/Home.tsx"]));
    assert!(!none.critical);
    assert!(none.critical_files.is_empty());
    assert!(!assess(&[]).critical);
}

#[test]
fn recovery_script_is_protected_and_nothing_else_is() {
    assert!(is_protected_path("scripts/neppy-recover.sh"));
    assert!(is_protected_path("./Scripts/Neppy-Recover.sh"));
    assert!(!is_protected_path("scripts/neppy-recover.sh.bak"));
    assert!(!is_protected_path("scripts/other.sh"));
    assert!(!is_protected_path("src/core/mod.rs"));
    // The recovery tool is also critical, so editing it needs a candidate.
    assert!(is_critical_path("scripts/neppy-recover.sh"));
}
