//! Attached-value `gh api` writes, workflow dispatch publish paths and the
//! package-manager / node entry points of the Debug command gate.

use super::*;

fn cfg() -> DebugModeConfig {
    DebugModeConfig {
        allow_git_push: true,
        allow_git_commit: true,
        allow_dependency_install: true,
        allow_system_commands: true,
        dangerous_commands_require_confirmation: false,
        ..DebugModeConfig::default()
    }
}

fn denied_as_publish(cmd: &str) -> bool {
    check_debug_command(cmd, &cfg()) == Deny(policy_release::DENY_MESSAGE.to_string())
}

#[test]
fn gh_api_write_forms_are_denied_in_every_spelling() {
    for c in [
        // separated forms (unchanged behaviour)
        "gh api repos/a/b/releases -X POST",
        "gh api repos/a/b/releases -f tag_name=v1",
        "gh api repos/a/b/releases -F draft=false",
        "gh api repos/a/b/releases --field tag_name=v1",
        "gh api repos/a/b/releases --raw-field body=x",
        "gh api repos/a/b/releases --input body.json",
        "gh api repos/a/b/releases --method DELETE",
        // attached-value forms
        "gh api repos/a/b/releases --field=tag_name=v1",
        "gh api repos/a/b/releases --raw-field=body=x",
        "gh api repos/a/b/releases --input=body.json",
        "gh api repos/a/b/releases --method=POST",
        "gh api repos/a/b/releases --method=delete",
        "gh api repos/a/b/releases -XPOST",
        "gh api repos/a/b/releases -XDELETE",
        "gh api repos/a/b/releases -ftag_name=v1",
        "gh api repos/a/b/releases -Fdraft=false",
        "gh api -XPOST repos/a/b/releases",
        "gh -R a/b api repos/a/b/releases --method=PATCH",
    ] {
        assert!(denied_as_publish(c), "`{c}` must be denied");
    }
}

#[test]
fn gh_api_read_forms_stay_allowed() {
    for c in [
        "gh api repos/a/b/releases",
        "gh api repos/a/b/releases -X GET",
        "gh api repos/a/b/releases --method GET",
        "gh api repos/a/b/releases --method=GET",
        "gh api repos/a/b/releases --method=get",
        "gh api repos/a/b/releases -XGET",
        "gh api repos/a/b/releases --jq .[0].tag_name",
        "gh api repos/a/b/releases --paginate",
        "gh api repos/a/b/pulls -q .[].number",
        "gh api repos/a/b/actions/workflows",
        "gh api repos/a/b/actions/runs --method=GET",
    ] {
        assert!(!denied_as_publish(c), "`{c}` must not be denied");
    }
}

#[test]
fn workflow_dispatch_publish_paths_are_denied() {
    for c in [
        "gh workflow run release-production.yml",
        "gh workflow run release-staging.yml --ref main",
        "gh workflow run promote-main-to-release.yml",
        "gh workflow run promote-main-to-release.yml -f version=1",
        "gh workflow run --ref main release-production.yml",
        "gh workflow run -r main Release.yml",
        "gh workflow run 'Promote main to release'",
        "gh workflow enable release-production.yml",
        "gh workflow enable promote-main-to-release.yml",
        "gh -R a/b workflow run release-staging.yml",
        "gh --repo a/b workflow run release-staging.yml",
        // a bare id cannot be checked against a name
        "gh workflow run 12345",
        // dispatch endpoints
        "gh api repos/a/b/actions/workflows/release-production.yml/dispatches -f ref=main",
        "gh api repos/a/b/actions/workflows/ci.yml/dispatches -X POST -f ref=main",
        "gh api repos/a/b/actions/workflows/ci.yml/dispatches --method=POST --field=ref=main",
        "gh api repos/a/b/actions/workflows/ci.yml/dispatches -XPOST",
        "gh api repos/a/b/actions/workflows/12345/dispatches --input body.json",
        "gh api repos/a/b/dispatches -f event_type=release",
        "gh api repos/a/b/dispatches --raw-field=event_type=x",
        "bash -c 'gh workflow run release-production.yml'",
        "echo | xargs gh workflow run promote-main-to-release.yml",
    ] {
        assert!(denied_as_publish(c), "`{c}` must be denied");
    }
}

#[test]
fn read_only_workflow_and_run_commands_stay_allowed() {
    for c in [
        "gh workflow list",
        "gh workflow list --all",
        "gh workflow view release-production.yml",
        "gh workflow view promote-main-to-release.yml --yaml",
        "gh workflow view 12345",
        "gh run list",
        "gh run list --workflow release-production.yml",
        "gh run view 123456",
        "gh run view 123456 --log",
        "gh workflow run ci.yml",
        "gh workflow run ci.yml --ref main -f note=hello",
        "gh workflow run 'Build and test'",
        "gh workflow enable ci.yml",
        "gh api repos/a/b/actions/workflows/release-production.yml",
        "gh api repos/a/b/actions/workflows/ci.yml/runs",
        "gh api repos/a/b/actions/workflows/ci.yml/dispatches",
        "gh api repos/a/b/actions/workflows/ci.yml/dispatches -X GET",
        "grep -rn 'gh workflow run' scripts",
        "cat .github/workflows/release-production.yml",
    ] {
        assert!(!denied_as_publish(c), "`{c}` must not be denied");
    }
}
