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

/// Round 4: EVERY shell call parks under the Pet companion origin — even a
/// command that only reads (bash parsing is too subtle to prove that). Plain
/// reads park as `privileged_command`.
#[test]
fn every_shell_command_parks() {
    for command in [
        "ls -la",
        "ls -la > /dev/null 2>&1",
        "grep -r 'rm -rf' src",
        "command -v cargo",
        "cat < input.txt",
        "cd src && ls",
        "find . -name '*.rs' -type f",
        "echo hello",
        "git status",
        "printf -v x %s y",
        "ls # rm -rf x",
        "cmd /C dir",
    ] {
        assert_eq!(shell(command), Some(C::PrivilegedCommand), "{command}");
    }
}

/// Round 3: classification depends on the tool NAME (and args), never on the
/// permission level or external effect a tool reports — internal deletes are
/// `delete`, executors privileged, and remote actions are labelled by the
/// action they run (W3), under the dispatchers' real names and argument keys.
#[test]
fn calls_are_classified_by_name_and_args() {
    let cases: Vec<(&str, serde_json::Value, Option<C>)> = vec![
        ("memory_forget", json!({}), Some(C::Delete)),
        ("goals_delete", json!({}), Some(C::Delete)),
        ("artifact_delete", json!({}), Some(C::Delete)),
        ("todo_remove", json!({}), Some(C::Delete)),
        ("shell", json!({"command": "rm -rf app"}), Some(C::Delete)),
        ("python_exec", json!({}), Some(C::PrivilegedCommand)),
        ("node_exec", json!({}), Some(C::PrivilegedCommand)),
        ("cron_add", json!({}), Some(C::SystemSettings)),
        (
            "shell",
            json!({"command": "git status"}),
            Some(C::PrivilegedCommand),
        ),
        ("memory_recall", json!({}), None),
        (
            "composio_execute",
            json!({"tool": "GMAIL_SEND_EMAIL"}),
            Some(C::SendMessage),
        ),
        (
            "composio_execute",
            json!({"tool": "GMAIL_FETCH_EMAILS"}),
            Some(C::SharePersonalInfo),
        ),
        (
            "composio_execute",
            json!({"tool": "GITHUB_CREATE_ISSUE"}),
            Some(C::Irreversible),
        ),
        ("composio_execute", json!({}), Some(C::Irreversible)),
        (
            "mcp_registry_tool_call",
            json!({"server_id": "s", "tool_name": "delete_issue"}),
            Some(C::Delete),
        ),
        (
            "mcp_registry_tool_call",
            json!({"server_id": "s", "tool_name": "list_issues"}),
            Some(C::SharePersonalInfo),
        ),
        (
            "mcp_call_tool",
            json!({"server": "s", "tool": "get_file"}),
            Some(C::SharePersonalInfo),
        ),
        (
            "mcp_registry_tool_call",
            json!({"server_id": "s"}),
            Some(C::Irreversible),
        ),
    ];
    for (tool, args, want) in cases {
        assert_eq!(pet_companion_high_risk(tool, &args), want, "{tool} {args}");
    }
}
