//! Pet desktop companion (user decision D4): which tool calls are high-risk.
//!
//! A Pet companion (`PetCompanion` origin) turn follows the user's normal
//! approval settings EXCEPT a high-risk action class (send message / email,
//! delete, purchase, publish, system settings, install software, privileged
//! commands, share personal info, irreversible changes), which ALWAYS parks for
//! confirmation — even with `auto_approve_all` on or the tool allowlisted.
//!
//! Two entry points, because the harness reaches the gate on two paths:
//! [`pet_companion_high_risk`] for a call that declares an external effect, and
//! [`pet_companion_internal_high_risk`] for one that does not (a Full-tier
//! `shell` write, `python_exec`, an internal `*_delete` / `memory_forget`, …),
//! which the approval middleware routes through the gate only when this says
//! it is high-risk.

use crate::neppy::pet::companion::types::ActionCategory;
use crate::neppy::tools::PermissionLevel;

use super::pet_shell::pet_companion_shell_class;

/// The high-risk [`ActionCategory`] of an `external_effect` call made under
/// Pet companion (`PetCompanion` origin), or `None` for an ordinary action
/// that follows the user's normal approval settings. A `Some` result makes the
/// gate park for confirmation **even with `auto_approve_all` on or the tool on
/// the `auto_approve` allowlist** (Pet Mode's standard: high-risk actions always
/// require confirmation).
///
/// The gate only sees the tool name and the *redacted* arguments, so the
/// classification is by name and by the few structural argument fields that
/// survive redaction (`command`, `operation`, `method`, `action`, `tool_slug`,
/// `tool`). Whenever that is not enough to prove an action is ordinary, it is
/// classified high-risk — the user gets asked, never surprised.
///
/// | Tool / pattern | Class |
/// |---|---|
/// | `shell`: `rm`/`rmdir`/`unlink`/`trash`/`shred`, `find … -delete`, `mv … /dev/null`, `git rm`, `git branch -d/-D`, `git tag -d`, `git stash drop/clear`, `git worktree remove`, `rsync --delete` | `delete` |
/// | `shell`: `git push` | `publish` |
/// | `shell`: `git reset`/`clean`/`checkout`/`restore`/`rebase`, a forced push, `mv`/`cp` without no-clobber, `dd`, `truncate`, `ln -f`, `tee` (not `-a`), a truncating `>` / `>|` / `&>` redirect to a real file | `irreversible` |
/// | `shell`: `mail`/`sendmail`/`mailx`/`msmtp` | `send_message` |
/// | `shell`: `defaults`/`launchctl`/`systemctl`/`crontab`/`scutil`/`networksetup`/`pmset`/`csrutil`/`spctl`/`tccutil`/`security` | `system_settings` |
/// | `shell`: command class `Install` (system / global package installs) | `install` |
/// | `shell`: command class `Network` (curl, ssh, scp, rsync, …) | `share_personal_info` |
/// | `shell`: class `Destructive` (mount, firewall, …); any interpreter, executor or wrapper (`sh`/`bash`/`zsh`/`fish`, `python*`, `perl`, `ruby`, `node`, `awk`, `osascript`, `open`, `eval`, `exec`, `source`, `sudo`, `doas`, `nice`, `nohup`, `timeout`, `command`, `env <cmd>`, `xargs`, `find -exec`, …); `kill*`, `chmod`/`chown`; hidden execution (`$(…)`, backticks, `<(…)`); a command named by a variable; an unparseable command (unbalanced quote, dangling redirect) or no command | `privileged_command` |
/// | `python_exec`, `node_exec` (arbitrary code the gate cannot inspect) | `privileged_command` |
/// | `npm_exec` | `install` |
/// | `http_request` / `curl` (data leaves the device; body is redacted) | `share_personal_info` (`delete` for method DELETE) |
/// | `git_operations`: `push` | `publish` |
/// | `git_operations`: `reset` / `checkout` / `clean` | `irreversible` |
/// | `composio_execute` (`tool`) / `mcp_call_tool` (`tool`) / `mcp_registry_tool_call` (`tool_name`): by the remote action's keywords below; a read-only verb (get/list/fetch/search/…) is ordinary; anything else on a remote account | `irreversible` |
/// | `use_skill` wrapper | the wrapped tool's class |
/// | `schedule`: cancel/remove/delete | `delete`; other mutations: `system_settings` |
/// | any name, by keyword token (first matching row wins): | |
/// | `pay payment(s) purchase buy checkout order transfer swap bridge wallet x402 trade stake withdraw deposit invoice charge refund billing subscription sell tip donate mint` | `purchase` |
/// | `delete remove rm trash purge erase destroy drop unlink uninstall wipe revoke forget clear` | `delete` |
/// | `send reply forward dm sms notify tweet comment invite respond call` | `send_message` |
/// | `publish deploy launch release post rollback visibility public unpublish` | `publish` |
/// | `share upload export link` | `share_personal_info` |
/// | `install installer upgrade` | `install` |
/// | `config configure settings setting autonomy permission(s) keyring credential(s) oauth password policy service daemon preferences cron schedule autostart` | `system_settings` |
/// | `sudo admin root chmod chown kill shutdown reboot` | `privileged_command` |
/// | `reset overwrite force cancel archive merge` | `irreversible` |
/// | ordinary (follows normal settings): `file_write`, `edit`, `apply_patch` (workspace edits), `git_operations` commit/add/stash/revert, `request_plan_review`, `learning_update_facet`, `learning_pin_facet`, the flow-draft tools (`propose_workflow`, `revise_workflow`, `edit_workflow`, `validate_workflow`, `suggest_workflows`), `storage_download_file`, and names led by a read-only verb | `None` |
/// | **any other external-effect tool** (unclassified) | `irreversible` |
pub(crate) fn pet_companion_high_risk(
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<ActionCategory> {
    pet_companion_classify(tool_name, args, 0, None)
}

/// The high-risk class of a companion call whose tool declares **no** external
/// effect for these args, or `None` when it may run under the user's normal
/// settings (which, for such a call, means it runs without a prompt — exactly
/// as in an interactive turn).
///
/// Same table as [`pet_companion_high_risk`] with two differences, both because
/// an internal tool is by default an ordinary workspace / memory operation:
/// an unclassified name is ordinary rather than `irreversible`, and a tool that
/// reports `permission` ≤ `ReadOnly` and whose name is led by a read-only verb
/// (`config_get`, `list_trash`) is ordinary even when a keyword matches. Every
/// specific branch (`shell`, `python_exec`, `node_exec`, `npm_exec`, `git_operations`,
/// remote actions, …) and every keyword class (`memory_forget`, `goals_delete`,
/// `artifact_delete` → `delete`; `cron_add` → `system_settings`) still applies.
pub(crate) fn pet_companion_internal_high_risk(
    tool_name: &str,
    args: &serde_json::Value,
    permission: PermissionLevel,
) -> Option<ActionCategory> {
    pet_companion_classify(tool_name, args, 0, Some(permission))
}

/// Tools that act only on the user's own workspace / drafts / profile, reversibly.
const PET_COMPANION_ORDINARY_TOOLS: &[&str] = &[
    "file_write",
    "edit",
    "apply_patch",
    "request_plan_review",
    "learning_update_facet",
    "learning_pin_facet",
    "propose_workflow",
    "revise_workflow",
    "edit_workflow",
    "validate_workflow",
    "suggest_workflows",
    "storage_download_file",
];

/// Leading verbs that mark a remote action / tool name as read-only.
const PET_COMPANION_READ_VERBS: &[&str] = &[
    "get", "list", "read", "search", "fetch", "find", "describe", "status", "show", "view",
    "query", "lookup", "browse", "preview", "count", "check", "validate", "inspect", "download",
];

/// Keyword → class, checked in this order (the first table with a hit wins).
const PET_COMPANION_KEYWORDS: &[(ActionCategory, &[&str])] = &[
    (
        ActionCategory::Purchase,
        &[
            "pay",
            "payment",
            "payments",
            "purchase",
            "buy",
            "checkout",
            "order",
            "transfer",
            "swap",
            "bridge",
            "wallet",
            "x402",
            "trade",
            "stake",
            "withdraw",
            "deposit",
            "invoice",
            "charge",
            "refund",
            "billing",
            "subscription",
            "sell",
            "tip",
            "donate",
            "mint",
        ],
    ),
    (
        ActionCategory::Delete,
        &[
            "delete",
            "remove",
            "rm",
            "trash",
            "purge",
            "erase",
            "destroy",
            "drop",
            "unlink",
            "uninstall",
            "wipe",
            "revoke",
            "forget",
            "clear",
        ],
    ),
    (
        ActionCategory::SendMessage,
        &[
            "send", "reply", "forward", "dm", "sms", "notify", "tweet", "comment", "invite",
            "respond", "call",
        ],
    ),
    (
        ActionCategory::Publish,
        &[
            "publish",
            "deploy",
            "launch",
            "release",
            "post",
            "rollback",
            "visibility",
            "public",
            "unpublish",
        ],
    ),
    (
        ActionCategory::SharePersonalInfo,
        &["share", "upload", "export", "link"],
    ),
    (
        ActionCategory::Install,
        &["install", "installer", "upgrade"],
    ),
    (
        ActionCategory::SystemSettings,
        &[
            "config",
            "configure",
            "settings",
            "setting",
            "autonomy",
            "permission",
            "permissions",
            "keyring",
            "credential",
            "credentials",
            "oauth",
            "password",
            "policy",
            "service",
            "daemon",
            "preferences",
            "cron",
            "schedule",
            "autostart",
        ],
    ),
    (
        ActionCategory::PrivilegedCommand,
        &[
            "sudo", "admin", "root", "chmod", "chown", "kill", "shutdown", "reboot",
        ],
    ),
    (
        ActionCategory::Irreversible,
        &["reset", "overwrite", "force", "cancel", "archive", "merge"],
    ),
];

/// `internal` is `None` for an external-effect call, or the tool's permission
/// level for a call with no external effect (see
/// [`pet_companion_internal_high_risk`]).
fn pet_companion_classify(
    tool_name: &str,
    args: &serde_json::Value,
    depth: usize,
    internal: Option<PermissionLevel>,
) -> Option<ActionCategory> {
    let name = tool_name.trim().to_ascii_lowercase();
    let str_arg = |key: &str| {
        args.get(key)
            .and_then(serde_json::Value::as_str)
            .map(|s| s.trim().to_ascii_lowercase())
    };
    match name.as_str() {
        // Runs a packed tool: classify what it wraps (bounded, so a nested
        // wrapper cannot recurse forever; an unreadable wrapper parks).
        crate::neppy::tools::toolpacks::USE_SKILL => {
            return match args.get("tool").and_then(serde_json::Value::as_str) {
                Some(inner) if depth < 2 => pet_companion_classify(
                    inner,
                    args.get("args").unwrap_or(&serde_json::Value::Null),
                    depth + 1,
                    // The wrapped tool's own level is unknown here.
                    internal.map(|_| PermissionLevel::Dangerous),
                ),
                _ => Some(ActionCategory::Irreversible),
            };
        }
        "shell" => return pet_companion_shell_class(args),
        "python_exec" | "node_exec" => return Some(ActionCategory::PrivilegedCommand),
        "npm_exec" => return Some(ActionCategory::Install),
        "http_request" | "curl" => {
            return Some(if str_arg("method").as_deref() == Some("delete") {
                ActionCategory::Delete
            } else {
                ActionCategory::SharePersonalInfo
            });
        }
        "git_operations" => {
            return match str_arg("operation").as_deref() {
                Some("push") => Some(ActionCategory::Publish),
                Some("reset" | "checkout" | "clean") => Some(ActionCategory::Irreversible),
                _ => None,
            };
        }
        "schedule" => {
            return match str_arg("action").as_deref() {
                Some("cancel" | "remove" | "delete") => Some(ActionCategory::Delete),
                _ => Some(ActionCategory::SystemSettings),
            };
        }
        // Remote actions are classified by the action they run, never by the
        // dispatcher's own name (`call` / `execute` say nothing about risk).
        "composio" | "composio_execute" | "mcp_call_tool" | "mcp_registry_tool_call" => {
            let slug = ["tool", "tool_slug", "action_name", "tool_name"]
                .iter()
                .find_map(|key| str_arg(key).filter(|s| !s.is_empty()));
            return match slug {
                Some(slug) => pet_companion_remote_action(&slug),
                None => Some(ActionCategory::Irreversible),
            };
        }
        _ => {}
    }
    if let Some(category) = pet_companion_keyword_class(&name) {
        let read_only = internal.is_some_and(|p| p <= PermissionLevel::ReadOnly);
        if read_only && led_by_read_verb(&name) {
            return None;
        }
        return Some(category);
    }
    if PET_COMPANION_ORDINARY_TOOLS.contains(&name.as_str()) || led_by_read_verb(&name) {
        return None;
    }
    if internal.is_some() {
        // No external effect and nothing high-risk in the name: an ordinary
        // internal operation, which runs as it would in an interactive turn.
        return None;
    }
    // Unclassified external effect: cannot prove it is ordinary, so ask.
    Some(ActionCategory::Irreversible)
}

/// A remote (Composio / MCP) action: keyword class, else ordinary when led by
/// a read-only verb, else `irreversible` (it runs on a remote account).
fn pet_companion_remote_action(slug: &str) -> Option<ActionCategory> {
    if let Some(category) = pet_companion_keyword_class(slug) {
        return Some(category);
    }
    if led_by_read_verb(slug) {
        return None;
    }
    Some(ActionCategory::Irreversible)
}

fn name_tokens(name: &str) -> impl Iterator<Item = String> + '_ {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase)
}

fn pet_companion_keyword_class(name: &str) -> Option<ActionCategory> {
    let tokens: Vec<String> = name_tokens(name).collect();
    PET_COMPANION_KEYWORDS
        .iter()
        .find(|(_, words)| tokens.iter().any(|t| words.contains(&t.as_str())))
        .map(|(category, _)| *category)
}

/// Whether a read-only verb appears among the first two tokens (`list_files`,
/// `GMAIL_FETCH_EMAILS` — an app prefix may come first).
fn led_by_read_verb(name: &str) -> bool {
    name_tokens(name)
        .take(2)
        .any(|t| PET_COMPANION_READ_VERBS.contains(&t.as_str()))
}

/// For the harness: the high-risk class of a call whose tool declares **no**
/// external effect, when the current turn is a Pet companion turn — `Some`
/// means the call must go through the approval gate anyway (which parks it,
/// whatever `auto_approve_all` / the allowlist say). `None` outside a companion
/// turn and for an ordinary call. Classifies the *redacted* arguments, exactly
/// what the gate itself will see, so both sides agree.
pub(crate) fn pet_companion_internal_gate_category(
    tool_name: &str,
    raw_args: &serde_json::Value,
    permission: PermissionLevel,
) -> Option<ActionCategory> {
    if !super::gate::is_pet_companion_turn() {
        return None;
    }
    let category = pet_companion_internal_high_risk(
        tool_name,
        &super::redact::redact_args(raw_args),
        permission,
    );
    if let Some(category) = category {
        tracing::info!(
            tool = tool_name,
            category = category.as_str(),
            "[approval::pet] companion call without a declared external effect is high-risk — \
             routing through the approval gate"
        );
    }
    category
}

#[cfg(test)]
#[path = "pet_classifier_tests.rs"]
pub(crate) mod tests;
