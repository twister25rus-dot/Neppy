//! Pet companion classifier (D4): the `shell` wrapper / executor / irreversible
//! git / overwrite cases (release audit B2), the internal-tool classification
//! that lets the harness route a call with no declared external effect through
//! the gate (B1), and remote actions classified by the action they run (W3).

use super::*;
use crate::neppy::pet::companion::types::ActionCategory as C;
use serde_json::json;

fn shell(command: &str) -> Option<C> {
    pet_companion_high_risk("shell", &json!({ "command": command }))
}

/// Every B2 example from the release audit, with its class. Shared with the
/// gate- and harness-level tests so all three layers pin the same list.
pub(crate) fn b2_shell_cases() -> Vec<(&'static str, C)> {
    vec![
        ("bash -c 'rm -rf build'", C::PrivilegedCommand),
        ("sh -c \"rm -rf build\"", C::PrivilegedCommand),
        ("zsh -c 'rm -rf build'", C::PrivilegedCommand),
        ("ls | xargs rm", C::PrivilegedCommand),
        ("env rm -rf build", C::PrivilegedCommand),
        (
            "python3 -c 'import os; os.remove(\"a\")'",
            C::PrivilegedCommand,
        ),
        ("perl -e 'unlink \"a\"'", C::PrivilegedCommand),
        (
            "node -e 'require(\"fs\").rmSync(\"a\")'",
            C::PrivilegedCommand,
        ),
        ("git checkout -- .", C::Irreversible),
        ("git branch -D feature", C::Delete),
        ("git stash drop", C::Delete),
        ("git stash clear", C::Delete),
        ("git rebase main", C::Irreversible),
        ("git rm notes.md", C::Delete),
        ("mv notes.md /dev/null", C::Delete),
        ("> notes.md", C::Irreversible),
        ("echo hi > notes.md", C::Irreversible),
        ("rm -rf ~/Neppy/projects/app", C::Delete),
        (
            "osascript -e 'tell application \"Mail\" to send'",
            C::PrivilegedCommand,
        ),
    ]
}

#[test]
fn every_b2_shell_example_is_high_risk() {
    for (command, want) in b2_shell_cases() {
        assert_eq!(shell(command), Some(want), "{command}");
    }
}

#[test]
fn interpreters_wrappers_and_executors_are_privileged() {
    for command in [
        "fish -c 'rm x'",
        "python script.py",
        "python3.12 -m http.server",
        "ruby -e 'File.delete(\"x\")'",
        "eval rm x",
        "exec rm x",
        "sudo ls",
        "doas ls",
        "nice rm x",
        "nohup rm x &",
        "timeout 5 rm x",
        "command rm x",
        "FOO=1 env BAR=2 rm x",
        "find . -name '*.log' -exec rm {} ;",
        "open https://example.test",
        "awk 'BEGIN{system(\"rm x\")}'",
        "source ./evil.sh",
        ". ./evil.sh",
        "$CMD x",
        "chmod -R 777 .",
        "cd /tmp && bash",
        "(rm x)",
        "{ rm x; }",
        "if true; then rm x; fi",
    ] {
        let class = shell(command);
        assert!(
            matches!(class, Some(C::PrivilegedCommand | C::Delete)),
            "{command}: {class:?}"
        );
    }
}

#[test]
fn irreversible_git_verbs_and_overwrites_park() {
    for (command, want) in [
        ("git -C repo reset --hard", C::Irreversible),
        ("git clean -fdx", C::Irreversible),
        ("git restore .", C::Irreversible),
        ("git switch -f main", C::Irreversible),
        ("git push --force-with-lease", C::Irreversible),
        ("git push origin +main", C::Irreversible),
        ("git push origin --delete old", C::Delete),
        ("git tag -d v1", C::Delete),
        ("git worktree remove ../wt", C::Delete),
        ("git reflog expire --all", C::Delete),
        ("git filter-branch --all", C::Irreversible),
        ("mv a.md b.md", C::Irreversible),
        ("cp -r src dst", C::Irreversible),
        ("dd if=/dev/zero of=disk.img", C::Irreversible),
        ("truncate -s 0 log.txt", C::Irreversible),
        ("ln -sf a b", C::Irreversible),
        ("cat a | tee b", C::Irreversible),
        ("cmd_output >| file", C::Irreversible),
        ("make &> build.log", C::Irreversible),
        ("make >& build.log", C::Irreversible),
        ("rsync -a --delete src/ dst/", C::Delete),
    ] {
        assert_eq!(shell(command), Some(want), "{command}");
    }
}

#[test]
fn ambiguous_commands_park() {
    for command in [
        "echo 'unterminated",
        "echo \"unterminated",
        "echo hi >",
        "echo trailing \\",
        "echo $(whoami)",
        "echo `whoami`",
        "diff <(ls a) <(ls b)",
    ] {
        assert_eq!(shell(command), Some(C::PrivilegedCommand), "{command}");
    }
}

/// The lexer respects quoting, so data that merely *mentions* a dangerous
/// token is not misread, and harmless redirects stay ordinary.
#[test]
fn ordinary_shell_commands_stay_ordinary() {
    for command in [
        "ls -la",
        "cargo build",
        "cargo test 2>&1",
        "cargo build > /dev/null 2>&1",
        "echo done >> build.log",
        "git status",
        "git commit -m 'remove the > sign; rm nothing'",
        "git add -A && git commit -m wip",
        "git stash",
        "git stash pop",
        "git switch main",
        "git branch feature",
        "git log --oneline | head -5",
        "mv -n a.md b.md",
        "cp --no-clobber a b",
        "grep -r 'rm -rf' src",
        "command -v cargo",
        "env",
        "time cargo build",
        "cat < input.txt",
    ] {
        assert_eq!(shell(command), None, "{command}");
    }
}

/// B1: a call with NO declared external effect — internal deletes are
/// `delete`, executors stay privileged — while ordinary internal work and
/// read-only tools that merely contain a keyword are not escalated.
#[test]
fn internal_calls_are_classified_for_the_harness() {
    use crate::neppy::tools::PermissionLevel as P;
    let cases: Vec<(&str, serde_json::Value, P, Option<C>)> = vec![
        ("memory_forget", json!({}), P::Write, Some(C::Delete)),
        // A delete tool that under-reports its level is still a delete.
        ("memory_forget", json!({}), P::ReadOnly, Some(C::Delete)),
        ("goals_delete", json!({}), P::Write, Some(C::Delete)),
        ("artifact_delete", json!({}), P::Write, Some(C::Delete)),
        ("todo_remove", json!({}), P::Write, Some(C::Delete)),
        (
            "shell",
            json!({"command": "rm -rf app"}),
            P::Execute,
            Some(C::Delete),
        ),
        (
            "python_exec",
            json!({}),
            P::Execute,
            Some(C::PrivilegedCommand),
        ),
        (
            "node_exec",
            json!({}),
            P::Execute,
            Some(C::PrivilegedCommand),
        ),
        ("cron_add", json!({}), P::Write, Some(C::SystemSettings)),
        // Ordinary internal operations run as in an interactive turn.
        ("memory_store", json!({}), P::Write, None),
        ("file_write", json!({}), P::Write, None),
        ("todo_write", json!({}), P::Write, None),
        ("spawn_subagent", json!({}), P::Write, None),
        ("shell", json!({"command": "cargo build"}), P::Execute, None),
        // Read-only tools led by a read verb are not escalated by a keyword.
        ("config_get", json!({}), P::ReadOnly, None),
        ("list_trash", json!({}), P::ReadOnly, None),
        ("memory_search", json!({}), P::ReadOnly, None),
    ];
    for (tool, args, level, want) in cases {
        assert_eq!(
            pet_companion_internal_high_risk(tool, &args, level),
            want,
            "{tool} {args} {level}"
        );
    }
    // The external-effect classifier keeps its stricter default.
    assert_eq!(
        pet_companion_high_risk("todo_write", &json!({})),
        Some(C::Irreversible)
    );
}

/// W3: Composio and MCP calls are classified by the remote action they run,
/// under the dispatchers' real tool names and argument keys.
#[test]
fn remote_actions_use_the_real_tool_names() {
    let cases: Vec<(&str, serde_json::Value, Option<C>)> = vec![
        (
            "composio_execute",
            json!({"tool": "GMAIL_FETCH_EMAILS"}),
            None,
        ),
        (
            "composio_execute",
            json!({"tool": "GITHUB_LIST_ISSUES"}),
            None,
        ),
        (
            "composio_execute",
            json!({"tool": "GMAIL_SEND_EMAIL"}),
            Some(C::SendMessage),
        ),
        (
            "composio_execute",
            json!({"tool": "GITHUB_CREATE_ISSUE"}),
            Some(C::Irreversible),
        ),
        ("composio_execute", json!({}), Some(C::Irreversible)),
        (
            "mcp_registry_tool_call",
            json!({"server_id": "s", "tool_name": "list_issues"}),
            None,
        ),
        (
            "mcp_registry_tool_call",
            json!({"server_id": "s", "tool_name": "delete_issue"}),
            Some(C::Delete),
        ),
        (
            "mcp_registry_tool_call",
            json!({"server_id": "s"}),
            Some(C::Irreversible),
        ),
        (
            "mcp_call_tool",
            json!({"server": "s", "tool": "get_file"}),
            None,
        ),
    ];
    for (tool, args, want) in cases {
        assert_eq!(pet_companion_high_risk(tool, &args), want, "{tool} {args}");
    }
}
