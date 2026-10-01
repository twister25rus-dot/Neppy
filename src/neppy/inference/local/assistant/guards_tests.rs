use std::path::{Path, PathBuf};

use crate::neppy::security::policy::AutonomyLevel;
use crate::neppy::security::{TrustedAccess, TrustedRoot};

use super::*;

/// A scratch machine: a home directory, a Neppy workspace, and projects.
struct Machine {
    _dir: tempfile::TempDir,
    home: PathBuf,
    workspace: PathBuf,
}

fn machine() -> Machine {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let home = base.join("home");
    let workspace = base.join("neppy-workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    Machine {
        _dir: dir,
        home,
        workspace,
    }
}

impl Machine {
    fn project(&self, rel: &str, git: bool) -> PathBuf {
        let path = self.home.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        if git {
            std::fs::create_dir_all(path.join(".git")).unwrap();
        }
        path
    }

    fn policy(&self, trusted: &[(&Path, TrustedAccess)], forbidden: &[&Path]) -> SecurityPolicy {
        SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: self.workspace.clone(),
            trusted_roots: trusted
                .iter()
                .map(|(p, access)| TrustedRoot {
                    path: p.to_string_lossy().into_owned(),
                    access: *access,
                })
                .collect(),
            forbidden_paths: forbidden
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            ..SecurityPolicy::default()
        }
    }

    fn check(
        &self,
        policy: &SecurityPolicy,
        root: &Path,
        edits: bool,
    ) -> std::result::Result<PathBuf, String> {
        let usage = if edits { RootUse::Edit } else { RootUse::Read };
        self.check_use(policy, root, usage)
    }

    fn check_use(
        &self,
        policy: &SecurityPolicy,
        root: &Path,
        usage: RootUse,
    ) -> std::result::Result<PathBuf, String> {
        validate_root_with_home(policy, &self.workspace, root, usage, Some(&self.home))
    }
}

const RW: TrustedAccess = TrustedAccess::ReadWrite;

#[test]
fn a_trusted_git_project_may_be_edited() {
    let m = machine();
    let project = m.project("work/app", true);
    let policy = m.policy(&[(&project, RW)], &[]);
    assert_eq!(m.check(&policy, &project, true), Ok(project));
}

#[test]
fn the_filesystem_root_the_home_directory_and_what_contains_it_are_refused() {
    let m = machine();
    let policy = m.policy(&[(&m.home, RW)], &[]);
    for edits in [false, true] {
        assert!(m.check(&policy, Path::new("/"), edits).is_err(), "/");
        let err = m.check(&policy, &m.home, edits).unwrap_err();
        assert!(err.contains("home directory"), "{err}");
        let parent = m.home.parent().unwrap();
        assert!(m.check(&policy, parent, edits).is_err(), "a parent of home");
    }
}

#[test]
fn a_root_without_a_git_repository_is_not_edited() {
    let m = machine();
    let project = m.project("work/plain", false);
    let policy = m.policy(&[(&project, RW)], &[]);
    let err = m.check(&policy, &project, true).unwrap_err();
    assert!(err.contains("git repository"), "{err}");
    // Reading it is fine: nothing changes.
    assert_eq!(m.check(&policy, &project, false), Ok(project));
}

#[test]
fn a_subdirectory_of_a_repository_counts_as_inside_it() {
    let m = machine();
    let repo = m.project("work/mono", true);
    let sub = repo.join("services/api");
    std::fs::create_dir_all(&sub).unwrap();
    let policy = m.policy(&[(&repo, RW)], &[]);
    assert_eq!(m.check(&policy, &sub, true), Ok(sub));
}

#[test]
fn a_repository_that_is_the_home_directory_is_not_a_project() {
    let m = machine();
    std::fs::create_dir_all(m.home.join(".git")).unwrap();
    let project = m.project("work/app", false);
    let policy = m.policy(&[(&project, RW)], &[]);
    let err = m.check(&policy, &project, true).unwrap_err();
    assert!(err.contains("home directory or above"), "{err}");
}

#[test]
fn a_root_outside_every_read_write_location_is_refused_for_edits() {
    let m = machine();
    let project = m.project("work/app", true);
    // No grant at all, and a read-only grant: both refuse a write root.
    let none = m.policy(&[], &[]);
    let err = m.check(&none, &project, true).unwrap_err();
    assert!(err.contains("trusted_roots"), "{err}");
    let read_only = m.policy(&[(&project, TrustedAccess::Read)], &[]);
    assert!(m.check(&read_only, &project, true).is_err());
    assert!(m.check(&read_only, &project, false).is_ok());
}

#[test]
fn system_and_persistence_locations_are_refused_for_edits() {
    let m = machine();
    // Under the default forbidden list, with no grant.
    let policy = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: m.workspace.clone(),
        ..SecurityPolicy::default()
    };
    for system in ["/usr/local", "/usr", "/etc", "/opt", "/var"] {
        let path = Path::new(system);
        if path.exists() {
            let err = m.check(&policy, path, true).unwrap_err();
            assert!(!err.is_empty(), "{system}");
        }
    }
    // `~/Library/LaunchAgents`: refused even when the user trusted it, and even
    // with a repository inside it.
    let agents = m.project("Library/LaunchAgents", true);
    for policy in [m.policy(&[], &[]), m.policy(&[(&agents, RW)], &[])] {
        let err = m.check(&policy, &agents, true).unwrap_err();
        assert!(
            err.contains("persistence") || err.contains("trusted_roots"),
            "{err}"
        );
    }
    let trusted = m.policy(&[(&agents, RW)], &[]);
    assert!(m
        .check(&trusted, &agents, true)
        .unwrap_err()
        .contains("persistence"));
    let nested = m.project("Library/LaunchDaemons/vendor", true);
    assert!(m
        .check(&m.policy(&[(&nested, RW)], &[]), &nested, true)
        .is_err());
}

#[test]
fn a_configured_forbidden_path_is_honoured_for_edits() {
    let m = machine();
    let project = m.project("work/app", true);
    let vault = project.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    // The project is trusted, but the user fenced off one directory inside it.
    let policy = m.policy(&[(&project, RW)], &[&vault]);
    let err = m.check(&policy, &vault, true).unwrap_err();
    assert!(err.contains("forbidden"), "{err}");
    assert!(m.check(&policy, &project, true).is_ok());
}

#[test]
fn a_trusted_root_carves_a_hole_in_a_broader_forbidden_path() {
    // The default list forbids /tmp, and scratch projects live under it: a
    // grant at or below the forbidden entry is the user's explicit exception.
    let m = machine();
    let project = m.project("work/app", true);
    let broad = m.home.join("work");
    let policy = m.policy(&[(&project, RW)], &[&broad]);
    assert!(m.check(&policy, &project, true).is_ok());
    let no_grant = m.policy(&[], &[&broad]);
    assert!(m.check(&no_grant, &project, true).is_err());
}

#[test]
fn the_workspace_is_never_a_project_root() {
    let m = machine();
    let policy = m.policy(&[(&m.workspace, RW)], &[]);
    assert!(m.check(&policy, &m.workspace, false).is_err());
    let inside = m.workspace.join("sub");
    std::fs::create_dir_all(&inside).unwrap();
    assert!(m.check(&policy, &inside, false).is_err());
    assert!(m
        .check(&policy, m.workspace.parent().unwrap(), false)
        .is_err());
}

#[test]
fn the_read_only_tier_refuses_edits_but_not_reading() {
    let m = machine();
    let project = m.project("work/app", true);
    let mut policy = m.policy(&[(&project, RW)], &[]);
    policy.autonomy = AutonomyLevel::ReadOnly;
    assert!(m.check(&policy, &project, true).is_err());
    assert!(m.check(&policy, &project, false).is_ok());
}

#[test]
fn a_missing_or_file_root_is_refused() {
    let m = machine();
    let policy = m.policy(&[], &[]);
    assert!(m.check(&policy, &m.home.join("nope"), false).is_err());
    let file = m.home.join("f.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(m.check(&policy, &file, false).is_err());
}

#[test]
fn sensitive_paths_are_the_ones_that_run_code_on_their_own() {
    let sensitive = [
        ".husky/pre-commit",
        ".githooks/pre-push",
        "app/.husky/_/husky.sh",
        ".vscode/tasks.json",
        ".vscode/settings.json",
        ".idea/workspace.xml",
        ".github/workflows/ci.yml",
        ".GitHub/Workflows/Release.yml",
        ".cargo/config.toml",
        ".cargo/config",
        "crates/x/.cargo/config.toml",
        "build.rs",
        "crates/x/build.rs",
        ".envrc",
        "package.json",
        "web/package.json",
        ".npmrc",
        ".git/hooks/pre-commit",
        ".devcontainer/devcontainer.json",
        // The wider denylist.
        "Makefile",
        "makefile",
        "GNUmakefile",
        "sub/Makefile",
        "rules.mk",
        "mk/common.MK",
        "pyproject.toml",
        "setup.py",
        "setup.cfg",
        "tests/conftest.py",
        "tox.ini",
        "noxfile.py",
        "Cargo.toml",
        "crates/x/Cargo.toml",
        ".yarn/releases/yarn-4.0.0.cjs",
        ".yarn/plugins/p.cjs",
        ".yarnrc.yml",
        ".pnpmfile.cjs",
        ".mise.toml",
        ".tool-versions",
        ".github/workflows/ci.yml",
        ".github/actions/setup/action.yml",
        ".github/CODEOWNERS",
        ".github/ISSUE_TEMPLATE/bug.md",
        "justfile",
        "Rakefile",
        "Gemfile",
        ".bashrc",
        ".zshrc",
        ".zshenv",
        ".profile",
        ".bash_profile",
        // A bare repository planted inside the project.
        "HEAD",
        "evil/HEAD",
        "evil/config",
        "evil.git/objects/x",
        "x/y.git/refs/heads/main",
    ];
    for path in sensitive {
        assert!(
            sensitive_reason(path).is_some(),
            "{path} should be sensitive"
        );
    }
    let ordinary = [
        "src/lib.rs",
        "README.md",
        ".cargo/notes.md",
        "docs/build.rs.md",
        "src/buildrs.rs",
        "package.json.md",
        "my-package.json",
        "src/vscode/mod.rs",
        "Cargo.lock.md",
        "src/make.rs",
        "docs/config.md",
        "src/config/mod.rs",
        "src/head.rs",
        "README.git.md",
    ];
    for path in ordinary {
        assert!(
            sensitive_reason(path).is_none(),
            "{path} should be ordinary"
        );
    }
}

fn edit_policy(m: &Machine, project: &Path) -> SecurityPolicy {
    m.policy(&[(project, RW)], &[])
}

#[test]
fn an_edit_to_a_sensitive_path_is_refused_unless_the_config_allows_it() {
    let m = machine();
    let project = m.project("work/app", true);
    let policy = edit_policy(&m, &project);
    let mut cfg = LocalAssistantConfig::default();
    for rel in [
        ".husky/pre-commit",
        ".vscode/tasks.json",
        "build.rs",
        ".envrc",
    ] {
        let abs = project.join(rel);
        let err = check_edit_target(&policy, &cfg, &project, rel, &abs).unwrap_err();
        assert!(err.contains("allow_sensitive_paths"), "{rel}: {err}");
    }
    assert!(check_edit_target(
        &policy,
        &cfg,
        &project,
        "src/lib.rs",
        &project.join("src/lib.rs")
    )
    .is_ok());
    cfg.allow_sensitive_paths = true;
    assert!(check_edit_target(
        &policy,
        &cfg,
        &project,
        ".husky/pre-commit",
        &project.join(".husky/pre-commit")
    )
    .is_ok());
}

#[test]
fn an_edit_target_is_judged_again_against_the_policy() {
    let m = machine();
    let project = m.project("work/app", true);
    let vault = project.join("vault");
    let cfg = LocalAssistantConfig::default();

    // A forbidden path added after the task was accepted.
    let fenced = m.policy(&[(&project, RW)], &[&vault]);
    let err = check_edit_target(
        &fenced,
        &cfg,
        &project,
        "vault/key.txt",
        &vault.join("key.txt"),
    )
    .unwrap_err();
    assert!(err.contains("forbidden"), "{err}");
    assert!(check_edit_target(
        &fenced,
        &cfg,
        &project,
        "src/a.rs",
        &project.join("src/a.rs")
    )
    .is_ok());

    // A grant that was withdrawn, or downgraded to read-only.
    let revoked = m.policy(&[], &[]);
    assert!(check_edit_target(
        &revoked,
        &cfg,
        &project,
        "src/a.rs",
        &project.join("src/a.rs")
    )
    .is_err());
    let read_only = m.policy(&[(&project, TrustedAccess::Read)], &[]);
    assert!(check_edit_target(
        &read_only,
        &cfg,
        &project,
        "src/a.rs",
        &project.join("src/a.rs")
    )
    .is_err());
}

#[test]
fn an_edit_target_that_resolves_into_a_persistence_location_is_refused() {
    let m = machine();
    let agents = m.project("Library/LaunchAgents", true);
    let policy = m.policy(&[(&agents, RW)], &[]);
    let cfg = LocalAssistantConfig::default();
    let err = check_edit_target(
        &policy,
        &cfg,
        &agents,
        "evil.plist",
        &agents.join("evil.plist"),
    )
    .unwrap_err();
    assert!(err.contains("persistence"), "{err}");
}

// ---- names are not trusted as written ------------------------------------

fn permissive() -> LocalAssistantConfig {
    LocalAssistantConfig::default()
}

#[test]
fn unicode_lookalikes_of_sensitive_names_are_refused() {
    let m = machine();
    let project = m.project("work/app", true);
    let policy = edit_policy(&m, &project);
    let cfg = permissive();
    // Create the real entries, so that on a volume that folds names the
    // lookalike resolves to something that exists.
    std::fs::create_dir_all(project.join(".husky")).unwrap();
    std::fs::write(project.join(".husky/pre-commit"), "#!/bin/sh\n").unwrap();
    std::fs::create_dir_all(project.join(".vscode")).unwrap();
    std::fs::write(project.join(".vscode/tasks.json"), "{}\n").unwrap();
    std::fs::create_dir_all(project.join(".github/workflows")).unwrap();
    std::fs::write(project.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
    std::fs::write(project.join("package.json"), "{}\n").unwrap();
    std::fs::write(project.join("build.rs"), "fn main() {}\n").unwrap();

    let lookalikes = [
        ".hu\u{17F}ky/pre-commit",
        ".v\u{17F}code/tasks.json",
        "package.j\u{17F}on",
        "build.r\u{17F}",
        ".github/workflow\u{17F}/ci.yml",
        ".github/workflows/ci.yml\u{17F}",
        ".HU\u{17F}KY/pre-commit",
        // The Kelvin sign folds to `k`.
        ".hus\u{212A}y/pre-commit",
        "pac\u{212A}age.json",
    ];
    for rel in lookalikes {
        let err = check_edit_target(&policy, &cfg, &project, rel, &project.join(rel))
            .expect_err(&format!("{rel:?} must be refused"));
        assert!(err.contains("allow_sensitive_paths"), "{rel:?}: {err}");
    }
}

#[test]
fn a_non_ascii_edit_path_is_refused_by_default() {
    let m = machine();
    let project = m.project("work/app", true);
    let policy = edit_policy(&m, &project);
    let mut cfg = permissive();
    for rel in [
        "docs/r\u{E9}sum\u{E9}.md",
        "\u{65E5}\u{672C}/a.rs",
        "src/caf\u{E9}/mod.rs",
    ] {
        let err = check_edit_target(&policy, &cfg, &project, rel, &project.join(rel))
            .expect_err(&format!("{rel:?} must be refused"));
        assert!(err.contains("non-ASCII"), "{rel:?}: {err}");
    }
    cfg.allow_sensitive_paths = true;
    let rel = "docs/r\u{E9}sum\u{E9}.md";
    assert!(check_edit_target(&policy, &cfg, &project, rel, &project.join(rel)).is_ok());
}

#[test]
fn the_on_disk_name_is_what_the_denylist_sees() {
    let m = machine();
    let project = m.project("work/app", true);
    std::fs::create_dir_all(project.join(".husky")).unwrap();
    std::fs::write(project.join(".husky/pre-commit"), "x").unwrap();
    // Wherever the lookup folds the given name onto the real entry (a
    // case-insensitive volume), the real name comes back.
    for given in [
        ".HUSKY/pre-commit",
        ".Husky/PRE-COMMIT",
        ".hu\u{17F}ky/pre-commit",
    ] {
        if project.join(given).exists() {
            assert_eq!(
                on_disk_rel(&project, Path::new(given)),
                ".husky/pre-commit",
                "{given:?} resolves on this volume"
            );
        }
    }
    // A name that resolves to nothing keeps its spelling.
    assert_eq!(
        on_disk_rel(&project, Path::new("src/new/file.rs")),
        "src/new/file.rs"
    );
    // The part that exists is reported by its on-disk name, the rest as given.
    assert_eq!(
        on_disk_rel(&project, Path::new(".husky/new.sh")),
        ".husky/new.sh"
    );
}

#[test]
fn a_case_variant_of_a_sensitive_name_is_refused_even_when_ascii() {
    let m = machine();
    let project = m.project("work/app", true);
    let policy = edit_policy(&m, &project);
    let cfg = permissive();
    std::fs::create_dir_all(project.join(".husky")).unwrap();
    for rel in [
        ".HUSKY/pre-commit",
        "PACKAGE.JSON",
        "makeFILE",
        "cargo.TOML",
        ".GitHub/Workflows/x.yml",
    ] {
        assert!(
            check_edit_target(&policy, &cfg, &project, rel, &project.join(rel)).is_err(),
            "{rel}"
        );
    }
}

#[test]
fn a_symlinked_directory_does_not_hide_a_sensitive_target() {
    let m = machine();
    let project = m.project("work/app", true);
    let policy = edit_policy(&m, &project);
    let cfg = permissive();
    std::fs::create_dir_all(project.join(".husky")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(project.join(".husky"), project.join("hooks_alias")).unwrap();
        let rel = "hooks_alias/pre-commit";
        let err = check_edit_target(&policy, &cfg, &project, rel, &project.join(rel)).unwrap_err();
        assert!(err.contains("resolves to"), "{err}");
    }
}

// ---- what a root is used for ---------------------------------------------

#[test]
fn a_root_for_a_test_command_gets_the_write_root_checks_but_not_the_git_one() {
    let m = machine();
    let plain = m.project("work/plain", false);
    let trusted = m.policy(&[(&plain, RW)], &[]);
    assert_eq!(
        m.check_use(&trusted, &plain, RootUse::Run),
        Ok(plain.clone())
    );
    // Not trusted: the command could write anywhere under it.
    let untrusted = m.policy(&[], &[]);
    let err = m.check_use(&untrusted, &plain, RootUse::Run).unwrap_err();
    assert!(err.contains("trusted_roots"), "{err}");
    // A forbidden path applies.
    let fenced = m.policy(&[(&plain, RW)], &[&plain]);
    assert!(
        m.check_use(&fenced, &plain, RootUse::Run).is_ok(),
        "the grant carves it out"
    );
    let broad = m.policy(&[], &[&plain]);
    assert!(m.check_use(&broad, &plain, RootUse::Run).is_err());
    // Reading needs none of it.
    assert!(m.check_use(&untrusted, &plain, RootUse::Read).is_ok());
    // Editing also wants the repository.
    assert!(m.check_use(&trusted, &plain, RootUse::Edit).is_err());
}

#[test]
fn the_use_of_a_root_follows_the_task_and_the_tier() {
    assert_eq!(RootUse::for_task(true, true, true), RootUse::Edit);
    assert_eq!(RootUse::for_task(true, false, true), RootUse::Edit);
    assert_eq!(RootUse::for_task(false, true, true), RootUse::Run);
    assert_eq!(RootUse::for_task(false, false, true), RootUse::Read);
    // A tier that cannot act runs and edits nothing.
    assert_eq!(RootUse::for_task(true, true, false), RootUse::Read);
    assert_eq!(RootUse::for_task(false, true, false), RootUse::Read);
}

#[test]
fn folding_covers_the_cases_apfs_resolves() {
    assert_eq!(fold("Package.J\u{17F}on"), "package.json");
    assert_eq!(fold(".HUS\u{212A}Y"), ".husky");
    assert_eq!(fold("Makefile"), "makefile");
    assert_eq!(
        first_non_ascii_component("a/b\u{17F}/c").as_deref(),
        Some("b\u{17F}")
    );
    assert_eq!(first_non_ascii_component("a/b/c"), None);
}
