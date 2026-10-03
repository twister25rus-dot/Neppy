//! Release re-review (round 2): the Pet companion classifier is an ALLOWLIST.
//!
//! * A: a shell command is ordinary only when every simple command is on the
//!   read-only allowlist — build tools, task runners, interpreters, wrappers,
//!   shell-state changes, obfuscation and paths in command position all park.
//! * B: an internal tool above `ReadOnly` is ordinary only when it is on the
//!   low-risk reversible allowlist (or a delegation tool); everything else
//!   parks. Composio per-action tools are classified by their slug.
//! * R4: the redactor's `<HOME>` placeholder is read as a path word.

use super::*;
use crate::neppy::pet::companion::types::ActionCategory as C;
use crate::neppy::tools::PermissionLevel as P;
use serde_json::json;

fn shell(command: &str) -> Option<C> {
    pet_companion_high_risk("shell", &json!({ "command": command }))
}

#[test]
fn everything_off_the_read_only_allowlist_parks() {
    for command in [
        // Build tools / task runners run arbitrary project code.
        "make",
        "make test",
        "npm run build",
        "npm test",
        "npx some-tool",
        "uvx ruff check",
        "cargo run",
        "cargo test",
        "cargo build",
        "go run .",
        "pytest -q",
        "just deploy",
        "./gradlew build",
        // Interpreters, wrappers, multiplexers.
        "busybox rm -rf x",
        "python3 -m http.server",
        "env",
        "env ls",
        "xargs echo",
        "time cargo build",
        // Paths / expansion in command position (symlink tricks, obfuscation).
        "./ls",
        "/tmp/x/cat notes",
        "~/bin/ls",
        "{ls,-la}",
        "$'\\x72m' -rf x",
        "l?",
        "c*t x",
        // Shell-state changes.
        "IFS=, ls",
        "FOO=1 ls",
        "PATH=/tmp/evil:$PATH; ls",
        "alias ls='rm -rf'",
        "trap 'rm -rf x' EXIT",
        "function f { rm x; }",
        "f() { rm x; }",
        "coproc rm x",
        // Git: config/alias tricks, hooks and writes.
        "git -c alias.st='!rm -rf x' st",
        "git -c core.pager=evil log",
        "git --exec-path=/tmp log",
        "git bisect run ./evil.sh",
        "git submodule foreach 'rm -rf .'",
        "git commit -m wip",
        "git add -A",
        "git stash",
        "git switch main",
        "git branch feature",
        "git diff --output=/tmp/x",
        "git diff --ext-diff",
        // Writes through otherwise read-only commands.
        "echo hi >> notes.md",
        "sort -o out.txt in.txt",
        "rg --pre ./evil.sh foo",
        "uniq in.txt out.txt",
        "tree -o out.txt",
        "find . -fprint out.txt",
        "mv -n a.md b.md",
        "cp --no-clobber a b",
        "touch x",
        "mkdir build",
    ] {
        assert!(shell(command).is_some(), "`{command}` must park");
    }
}

#[test]
fn recognisable_commands_keep_their_precise_label() {
    for (command, want) in [
        ("rm -rf build", C::Delete),
        ("FOO=1 rm x", C::Delete),
        ("git push origin main", C::Publish),
        ("git reset --hard", C::Irreversible),
        ("echo hi > notes.md", C::Irreversible),
        ("curl https://x.test", C::SharePersonalInfo),
        ("brew install jq", C::Install),
        ("defaults write x y", C::SystemSettings),
        ("cargo build", C::PrivilegedCommand),
    ] {
        assert_eq!(shell(command), Some(want), "{command}");
    }
}

/// R4: the gate classifies *redacted* args; a home path redacted to `<HOME>…`
/// must stay an ordinary path word, not turn into an input redirect.
#[test]
fn redacted_home_paths_do_not_change_the_class() {
    let raw = json!({ "command": "ls -la /Users/alice/projects/app" });
    let redacted = super::super::redact::redact_args(&raw);
    let command = redacted["command"].as_str().unwrap();
    assert!(
        command.contains(super::super::pet_shell::HOME_PLACEHOLDER),
        "redactor placeholder drifted: {command}"
    );
    assert_eq!(
        pet_companion_high_risk("shell", &redacted),
        None,
        "{command}"
    );
    assert_eq!(pet_companion_high_risk("shell", &raw), None);
    let rm = super::super::redact::redact_args(&json!({ "command": "rm -rf /Users/alice/x" }));
    assert_eq!(pet_companion_high_risk("shell", &rm), Some(C::Delete));
}

#[test]
fn internal_tools_above_read_only_park_unless_allowlisted() {
    let cases: Vec<(&str, serde_json::Value, P, Option<C>)> = vec![
        // Allowlisted low-risk, reversible internal work.
        ("memory_store", json!({}), P::Write, None),
        ("save_preference", json!({}), P::Write, None),
        ("pet_note", json!({}), P::Write, None),
        ("todo_add", json!({}), P::Write, None),
        ("file_write", json!({}), P::Write, None),
        ("apply_patch", json!({}), P::Write, None),
        ("spawn_subagent", json!({}), P::Write, None),
        ("delegate_researcher", json!({}), P::Write, None),
        ("ask_user_clarification", json!({}), P::Write, None),
        // Cannot act: ordinary.
        ("memory_recall", json!({}), P::ReadOnly, None),
        ("some_reader", json!({}), P::ReadOnly, None),
        // Everything else above ReadOnly parks.
        (
            "some_internal_writer",
            json!({}),
            P::Write,
            Some(C::Irreversible),
        ),
        ("goals_add", json!({}), P::Write, Some(C::Irreversible)),
        ("goal_set", json!({}), P::Write, Some(C::Irreversible)),
        ("goal_set", json!({}), P::ReadOnly, Some(C::Irreversible)),
        ("cron_add", json!({}), P::Write, Some(C::SystemSettings)),
        (
            "git_operations",
            json!({"operation": "stash", "action": "drop"}),
            P::Write,
            Some(C::Delete),
        ),
        (
            "git_operations",
            json!({"operation": "stash", "action": "clear"}),
            P::Write,
            Some(C::Delete),
        ),
        (
            "git_operations",
            json!({"operation": "commit"}),
            P::Write,
            None,
        ),
        // web3 execution signs and broadcasts, whatever level it reports.
        (
            "web3_dapp_execute",
            json!({}),
            P::ReadOnly,
            Some(C::Purchase),
        ),
        (
            "web3_swap_execute",
            json!({}),
            P::ReadOnly,
            Some(C::Purchase),
        ),
        ("web3_dapp_call", json!({}), P::ReadOnly, None),
        // Composio per-action tools: by slug, unclassified = irreversible.
        (
            "GMAIL_SEND_EMAIL",
            json!({}),
            P::Write,
            Some(C::SendMessage),
        ),
        ("GMAIL_FETCH_EMAILS", json!({}), P::Write, None),
        (
            "GITHUB_CREATE_ISSUE",
            json!({}),
            P::Write,
            Some(C::Irreversible),
        ),
        ("NOTION_DELETE_PAGE", json!({}), P::Write, Some(C::Delete)),
    ];
    for (tool, args, level, want) in cases {
        assert_eq!(
            pet_companion_internal_high_risk(tool, &args, level),
            want,
            "{tool} {args} {level}"
        );
    }
}
