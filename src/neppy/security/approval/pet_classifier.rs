//! Pet desktop companion (user decision D4): which tool calls are ordinary.
//!
//! **A closed, reviewed allowlist of tool names.** Under the Pet companion
//! origin — a hand-off run, every sub-agent it delegates to, and every
//! follow-up turn on its thread — a call is ordinary ONLY when its tool name is
//! in [`PET_COMPANION_ALLOWED_TOOLS`] (plus the argument-checked cases: a
//! `shell` command that passes the read-only shell rule, `git_operations`
//! status/diff/log, and blocking delegation). Everything else parks for
//! confirmation, even with `auto_approve_all` on or the tool allowlisted —
//! **whatever permission level or external effect the tool reports about
//! itself**. Self-reporting is not trusted: `Tool::permission_level` defaults
//! to `ReadOnly`, so an acting tool that never overrides it (browser
//! automation, push notifications, …) would otherwise read as harmless.
//!
//! A parked call carries the sharpest [`ActionCategory`] recognisable from its
//! name and the few structural arguments that survive redaction, and
//! `irreversible` otherwise. The middleware and the gate call the same
//! function ([`pet_companion_high_risk`]) with the same inputs (tool name +
//! redacted arguments, in the same task, so the same ambient turn mode).

use crate::neppy::pet::companion::types::ActionCategory;

use super::pet_shell::pet_companion_shell_class;

/// The reviewed allowlist. Each entry was inspected for side effects; none
/// writes outside the agent's own per-thread scratch, sends data off the
/// device, or runs a program the gate cannot see:
///
/// * `file_read`, `glob`, `grep` — workspace reads (path policy enforced in the
///   tool; `grep` runs no shell);
/// * `read_workspace_state` — git state through a hardened `git` (system and
///   global config closed, repo config neutralised, external diff removed);
/// * `retrieve_tool_output`, `tinyjuice_retrieve` (+ alias) — re-read output
///   this run already produced;
/// * `current_time`, `resolve_time` — clock / date arithmetic;
/// * `memory_recall`, `memory_hybrid_search`, `memory_vector_search`,
///   `memory_chunk_context` — local memory reads (no write-back);
/// * `pet_context`, `pet_recent_memory` — Pet reads; `pet_note` — records a
///   Pet note (the Pet's own inbox, reviewed by the user before anything acts
///   on it);
/// * `todo_list`, `todo_add`, `todo_edit`, `todo_update_status`, `todo_replace`,
///   `todo_decide_plan`, `goal_get` — the thread's own task list / goal read;
/// * `ask_user_clarification`, `request_plan_review`, `plan_exit` — turn control
///   that hands the decision back to the user;
/// * `load_skill` — loads a packed tool group's schemas (running one goes
///   through `use_skill`, classified by the tool it wraps);
/// * `continue_subagent` — resumes a paused child in this turn, under this
///   origin.
pub(crate) const PET_COMPANION_ALLOWED_TOOLS: &[&str] = &[
    "file_read",
    "glob",
    "grep",
    "read_workspace_state",
    "retrieve_tool_output",
    "current_time",
    "resolve_time",
    "memory_recall",
    "memory_hybrid_search",
    "memory_vector_search",
    "memory_chunk_context",
    "pet_context",
    "pet_recent_memory",
    "pet_note",
    "todo_list",
    "todo_add",
    "todo_edit",
    "todo_update_status",
    "todo_replace",
    "todo_decide_plan",
    "goal_get",
    "ask_user_clarification",
    "request_plan_review",
    "plan_exit",
    "load_skill",
    "continue_subagent",
];

/// The [`ActionCategory`] a Pet companion call parks under, or `None` when it
/// is ordinary (see the module docs). Used by the approval middleware for
/// every call of a Pet companion turn and by the gate itself.
///
/// | Tool / pattern | Class |
/// |---|---|
/// | name in [`PET_COMPANION_ALLOWED_TOOLS`] (or the CCR recovery tool) | ordinary |
/// | `shell`: ordinary only when every simple command passes the read-only shell rule (`pet_shell` / `pet_shell_readonly`); otherwise its sharpest label (`rm` → `delete`, `git push` → `publish`, `> file` → `irreversible`, curl → `share_personal_info`, …) or `privileged_command` | |
/// | `git_operations`: `status` / `diff` / `log` ordinary; `push` → `publish`; `stash` drop/clear → `delete`; anything else → `irreversible` | |
/// | `spawn_subagent`: ordinary only when it runs blocking (Chat turn, or `blocking: true`); `delegate_*` / an agent's `delegate_name`: only in a Chat turn — otherwise `irreversible` (a detached worker) | |
/// | `use_skill` | the wrapped tool's class |
/// | `python_exec`, `node_exec`, `browser*` | `privileged_command` |
/// | `npm_exec` | `install` |
/// | `http_request` / `curl` | `share_personal_info` (`delete` for DELETE) |
/// | `web_fetch`, `web_*`, any remaining `*search*` tool (a query leaves the device) | `share_personal_info` |
/// | `pushover` | `send_message` |
/// | `schedule`: cancel/remove/delete → `delete`, else `system_settings`; `cron_*` | `system_settings` |
/// | `web3_*` | `purchase` |
/// | Composio / MCP remote actions (`composio_execute`, `mcp_call_tool`, `mcp_registry_tool_call`, per-action `UPPER_SNAKE` tools) | keyword class, else `share_personal_info` for a read verb, else `irreversible` |
/// | any other name: keyword class (table below), else | `irreversible` |
/// | keywords: `pay payment(s) purchase buy checkout order transfer swap bridge wallet x402 trade stake withdraw deposit invoice charge refund billing subscription sell tip donate mint dapp` | `purchase` |
/// | `delete remove rm trash purge erase destroy drop unlink uninstall wipe revoke forget clear` | `delete` |
/// | `send reply forward dm sms notify tweet comment invite respond call` | `send_message` |
/// | `publish deploy launch release post rollback visibility public unpublish` | `publish` |
/// | `share upload export link` | `share_personal_info` |
/// | `install installer upgrade` | `install` |
/// | `config configure settings setting autonomy permission(s) keyring credential(s) oauth password policy service daemon preferences cron schedule autostart` | `system_settings` |
/// | `sudo admin root chmod chown kill shutdown reboot` | `privileged_command` |
/// | `reset overwrite force cancel archive merge` | `irreversible` |
pub(crate) fn pet_companion_high_risk(
    tool_name: &str,
    args: &serde_json::Value,
) -> Option<ActionCategory> {
    pet_companion_classify(tool_name, args, 0)
}

/// Leading verbs that mark a remote action name as a read.
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
            "dapp",
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

/// A delegation tool: `spawn_subagent`'s synthesised siblings, named
/// `delegate_*` or by an agent's `delegate_name` override.
fn is_delegation_tool(name: &str) -> bool {
    name.starts_with("delegate_")
        || crate::neppy::agent::harness::definition::AgentDefinitionRegistry::global().is_some_and(
            |reg| {
                reg.list()
                    .iter()
                    .any(|def| def.delegate_name.as_deref() == Some(name))
            },
        )
}

/// A Composio per-action tool (`ComposioActionTool`) is named by its action
/// slug, `UPPER_SNAKE` (`GMAIL_SEND_EMAIL`); native tools are `lower_snake`.
fn is_composio_action_slug(name: &str) -> bool {
    name.contains('_')
        && name.chars().any(|c| c.is_ascii_uppercase())
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Whether the current turn runs in Chat mode, where delegation is blocking
/// (`delegate_*` via `dispatch.rs`, `spawn_subagent` forced blocking): the
/// child runs inside this turn, under this origin, and returns here.
fn chat_turn() -> bool {
    crate::neppy::threads::mode::current_turn_mode()
        == Some(crate::neppy::threads::mode::ThreadMode::Chat)
}

fn pet_companion_classify(
    tool_name: &str,
    args: &serde_json::Value,
    depth: usize,
) -> Option<ActionCategory> {
    let trimmed = tool_name.trim();
    // Composio per-action tools run a remote action: never ordinary.
    if is_composio_action_slug(trimmed) {
        return Some(remote_action_label(&trimmed.to_ascii_lowercase()));
    }
    let name = trimmed.to_ascii_lowercase();
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
                ),
                _ => Some(ActionCategory::Irreversible),
            };
        }
        "shell" => return pet_companion_shell_class(args),
        "git_operations" => {
            return match str_arg("operation").as_deref() {
                // Even read-only git verbs honour a repository's own config
                // (core.fsmonitor, diff drivers), which can run a program.
                // `read_workspace_state` is the hardened read path; git
                // itself always asks under the Pet.
                Some("status" | "diff" | "log") => Some(ActionCategory::PrivilegedCommand),
                Some("push") => Some(ActionCategory::Publish),
                Some("stash") if matches!(str_arg("action").as_deref(), Some("drop" | "clear")) => {
                    Some(ActionCategory::Delete)
                }
                _ => Some(ActionCategory::Irreversible),
            };
        }
        "spawn_subagent" => {
            let blocking = args.get("blocking").and_then(serde_json::Value::as_bool) == Some(true);
            return (!(blocking || chat_turn())).then_some(ActionCategory::Irreversible);
        }
        n if is_delegation_tool(n) => {
            return (!chat_turn()).then_some(ActionCategory::Irreversible);
        }
        n if PET_COMPANION_ALLOWED_TOOLS.contains(&n)
            || crate::neppy::inference::tokenjuice::is_recovery_tool(n) =>
        {
            return None;
        }
        _ => {}
    }
    Some(parked_label(&name, &str_arg))
}

/// The sharpest recognisable class for a call that is not ordinary.
fn parked_label(name: &str, str_arg: &dyn Fn(&str) -> Option<String>) -> ActionCategory {
    use ActionCategory as C;
    match name {
        "python_exec" | "node_exec" => C::PrivilegedCommand,
        n if n == "browser" || n.starts_with("browser_") => C::PrivilegedCommand,
        "npm_exec" => C::Install,
        "http_request" | "curl" => {
            if str_arg("method").as_deref() == Some("delete") {
                C::Delete
            } else {
                C::SharePersonalInfo
            }
        }
        "pushover" => C::SendMessage,
        "schedule" => match str_arg("action").as_deref() {
            Some("cancel" | "remove" | "delete") => C::Delete,
            _ => C::SystemSettings,
        },
        n if n.starts_with("web3_") => C::Purchase,
        // Remote actions are classified by the action they run, never by the
        // dispatcher's own name (`call` / `execute` say nothing about risk).
        "composio" | "composio_execute" | "mcp_call_tool" | "mcp_registry_tool_call" => {
            match ["tool", "tool_slug", "action_name", "tool_name"]
                .iter()
                .find_map(|key| str_arg(key).filter(|s| !s.is_empty()))
            {
                Some(slug) => remote_action_label(&slug),
                None => C::Irreversible,
            }
        }
        // A web fetch or a search query sends data off the device.
        n if n.starts_with("web_") || n.contains("search") => C::SharePersonalInfo,
        _ => pet_companion_keyword_class(name).unwrap_or(C::Irreversible),
    }
}

/// A remote (Composio / MCP) action's class: keyword class, else
/// `share_personal_info` for a read (account data flows to the model), else
/// `irreversible` (it acts on a remote account).
fn remote_action_label(slug: &str) -> ActionCategory {
    pet_companion_keyword_class(slug).unwrap_or(if led_by_read_verb(slug) {
        ActionCategory::SharePersonalInfo
    } else {
        ActionCategory::Irreversible
    })
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

/// For the harness: the class a Pet companion call parks under, or `None` when
/// it is ordinary or the turn is not a Pet companion turn. Classifies the
/// *redacted* arguments with [`pet_companion_high_risk`] — exactly what the
/// gate will see and run — so the middleware and the gate always agree.
pub(crate) fn pet_companion_gate_category(
    tool_name: &str,
    raw_args: &serde_json::Value,
) -> Option<ActionCategory> {
    if !super::gate::is_pet_companion_turn() {
        return None;
    }
    let category = pet_companion_high_risk(tool_name, &super::redact::redact_args(raw_args));
    if let Some(category) = category {
        tracing::info!(
            tool = tool_name,
            category = category.as_str(),
            "[approval::pet] companion call is not on the Pet allowlist — routing through the \
             approval gate"
        );
    }
    category
}

#[cfg(test)]
#[path = "pet_classifier_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "pet_classifier_allowlist_tests.rs"]
mod allowlist_tests;
