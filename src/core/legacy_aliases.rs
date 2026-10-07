//! Server-side legacy RPC method aliases.
//!
//! Mirrors the frontend's `LEGACY_METHOD_ALIASES` table in
//! `app/src/services/rpcMethods.ts`. The frontend rewrites outgoing method
//! names for clients that just updated; this module rewrites incoming
//! method names for clients that haven't updated yet (older shipped bundles
//! in the wild). Together they form a symmetric migration safety net:
//! either side can be the one that's behind, and the call still resolves.
//!
//! When adding or removing an entry here, keep
//! `app/src/services/rpcMethods.ts:LEGACY_METHOD_ALIASES` in sync. The two
//! tables are intentionally identical: the same legacy → canonical map
//! applied at both ends of the wire.
//!
//! The rewrite is a pure key-to-key lookup. No domain branches, no
//! parameter inspection — if a method isn't in the table, it passes through
//! untouched.
//!
//! ## The `openhuman.` -> `neppy.` rebrand
//!
//! The canonical method prefix is `neppy.`. The old `openhuman.` prefix is a
//! permanent legacy alias: [`normalize_rpc_method`] rewrites it before the
//! table is consulted, so the table below is written entirely in the canonical
//! `neppy.` spelling and one entry covers both prefixes.

/// Legacy → canonical RPC method name pairs.
///
/// Order doesn't matter for correctness, but is kept alphabetical by legacy
/// key for easier diffing against the frontend table.
const LEGACY_ALIASES: &[(&str, &str)] = &[
    // #3565: old desktop clients called the channels controller with a dotted
    // namespace/function spelling before the canonical
    // `neppy.<namespace>_<function>` form was established.
    ("channels.list", "neppy.channels_list"),
    // MCP clients — old method names that appeared in Sentry (CORE-RUST-DR/DS/DT/DV/DW).
    // Callers used dotted namespace, bare `mcp_list`, `mcp_servers_list`, and
    // `mcp_clients_list` before the canonical `mcp_clients_installed_list` was
    // introduced in PR #2409. `tool_registry_call` was an early mis-spelling of
    // `mcp_clients_tool_call` that shipped in at least one older bundle.
    // `mcp_clients.list` sorts before all `neppy.*` entries (m < n).
    ("mcp_clients.list", "neppy.mcp_clients_installed_list"),
    ("neppy.channels.list", "neppy.channels_list"),
    (
        "neppy.get_analytics_settings",
        "neppy.config_get_analytics_settings",
    ),
    (
        "neppy.get_composio_trigger_settings",
        "neppy.config_get_composio_trigger_settings",
    ),
    (
        "neppy.get_dashboard_settings",
        "neppy.config_get_dashboard_settings",
    ),
    ("neppy.get_config", "neppy.config_get"),
    ("neppy.get_runtime_flags", "neppy.config_get_runtime_flags"),
    ("neppy.mcp_clients_list", "neppy.mcp_clients_installed_list"),
    ("neppy.mcp_list", "neppy.mcp_clients_installed_list"),
    ("neppy.mcp_servers_list", "neppy.mcp_clients_installed_list"),
    ("neppy.ping", "core.ping"),
    (
        "neppy.set_browser_allow_all",
        "neppy.config_set_browser_allow_all",
    ),
    ("neppy.tool_registry_call", "neppy.mcp_clients_tool_call"),
    // #3294: old desktop bundles called the tool-registry diagnostics
    // controller with the dotted `tool_registry.diagnostics` spelling, before
    // the canonical `neppy.<namespace>_<function>` form
    // (`neppy.tool_registry_diagnostics`) was established. Without this
    // alias the Tool Policy diagnostics panel's RPC failed with "unknown
    // method" on those clients.
    (
        "tool_registry.diagnostics",
        "neppy.tool_registry_diagnostics",
    ),
    (
        "neppy.update_analytics_settings",
        "neppy.config_update_analytics_settings",
    ),
    (
        "neppy.update_autonomy_settings",
        "neppy.config_update_autonomy_settings",
    ),
    (
        "neppy.update_browser_settings",
        "neppy.config_update_browser_settings",
    ),
    (
        "neppy.update_composio_trigger_settings",
        "neppy.config_update_composio_trigger_settings",
    ),
    (
        "neppy.update_local_ai_settings",
        "neppy.inference_update_local_settings",
    ),
    (
        "neppy.update_memory_settings",
        "neppy.config_update_memory_settings",
    ),
    (
        "neppy.update_model_settings",
        "neppy.inference_update_model_settings",
    ),
    (
        "neppy.update_runtime_settings",
        "neppy.config_update_runtime_settings",
    ),
    (
        "neppy.workspace_onboarding_flag_exists",
        "neppy.config_workspace_onboarding_flag_exists",
    ),
    (
        "neppy.workspace_onboarding_flag_set",
        "neppy.config_workspace_onboarding_flag_set",
    ),
    (
        "neppy.local_ai_apply_preset",
        "neppy.inference_apply_preset",
    ),
    ("neppy.local_ai_agent_chat", "neppy.inference_agent_chat"),
    (
        "neppy.local_ai_agent_chat_simple",
        "neppy.inference_agent_chat_simple",
    ),
    (
        "neppy.local_ai_assets_status",
        "neppy.inference_assets_status",
    ),
    (
        "neppy.local_ai_device_profile",
        "neppy.inference_device_profile",
    ),
    ("neppy.local_ai_diagnostics", "neppy.inference_diagnostics"),
    (
        "neppy.local_ai_download_asset",
        "neppy.inference_download_asset",
    ),
    (
        "neppy.local_ai_downloads_progress",
        "neppy.inference_downloads_progress",
    ),
    (
        "neppy.local_ai_install_piper",
        "neppy.inference_install_piper",
    ),
    (
        "neppy.local_ai_piper_install_status",
        "neppy.inference_piper_install_status",
    ),
    // bare `health_snapshot` (no namespace prefix) was used by older clients
    // before the canonical `neppy.health_snapshot` form was established.
    ("health_snapshot", "neppy.health_snapshot"),
    // Dotted / bare health probes from older clients and SDK callers (#3566,
    // Sentry CORE-2C). The canonical method is `neppy.health_snapshot`
    // (namespace `health`, function `snapshot`); these legacy spellings fell
    // through to the unknown-method path and produced Sentry noise. There is no
    // distinct `status`/`get` health handler — the snapshot already carries the
    // health verdict (`healthy`/`degraded`/`critical_unhealthy`), so all four
    // variants alias to the snapshot.
    ("health", "neppy.health_snapshot"),
    ("health.get", "neppy.health_snapshot"),
    ("health.snapshot", "neppy.health_snapshot"),
    ("health.status", "neppy.health_snapshot"),
    // `neppy.system_info` was used by older clients / SDK callers before
    // the method was namespaced under `health` as `neppy.health_system_info`.
    // Sentry CORE-RUST-G0 — https://sentry.tinyhumans.ai/organizations/tinyhumans/issues/6340/
    ("neppy.system_info", "neppy.health_system_info"),
    ("neppy.inference_embed", "neppy.embeddings_embed"),
    ("neppy.local_ai_presets", "neppy.inference_presets"),
    (
        "neppy.local_ai_test_connection",
        "neppy.inference_test_connection",
    ),
    ("neppy.local_ai_transcribe", "neppy.inference_transcribe"),
    (
        "neppy.local_ai_transcribe_bytes",
        "neppy.inference_transcribe_bytes",
    ),
    ("neppy.local_ai_tts", "neppy.inference_tts"),
    ("neppy.providers_list_models", "neppy.inference_list_models"),
];

/// Returns the server-side legacy → canonical RPC alias table.
///
/// Keep this as the single Rust metadata source for alias consumers and tests;
/// drift guards compare it with the frontend catalog in
/// `app/src/services/rpcMethods.ts`.
fn legacy_aliases() -> &'static [(&'static str, &'static str)] {
    LEGACY_ALIASES
}

/// Canonical RPC method prefix (`neppy.<namespace>_<function>`).
pub const RPC_METHOD_PREFIX: &str = "neppy.";

/// Pre-rebrand RPC method prefix. Still accepted on input forever — see
/// [`normalize_rpc_method`] — but never emitted by the core.
pub const LEGACY_RPC_METHOD_PREFIX: &str = "openhuman.";

/// Rewrites the pre-rebrand `openhuman.` method prefix to the canonical
/// `neppy.` one. Every other name (including already-canonical `neppy.*` and
/// bare `core.*` names) is returned borrowed and unchanged.
///
/// This is the single normalisation point for the product prefix: the
/// transport-level entry points (`jsonrpc::invoke_method`, `dispatch`), the
/// registry lookups in `core::all` (which MCP tool dispatch and in-process
/// embedders call directly), and persisted-name comparisons all go through it,
/// so a client, config file, cron payload or allow-list written against the old
/// spelling keeps working. It does **not** consult the legacy alias table; use
/// [`resolve_legacy`] for that.
pub fn normalize_rpc_method(method: &str) -> std::borrow::Cow<'_, str> {
    match method.strip_prefix(LEGACY_RPC_METHOD_PREFIX) {
        Some(rest) => std::borrow::Cow::Owned(format!("{RPC_METHOD_PREFIX}{rest}")),
        None => std::borrow::Cow::Borrowed(method),
    }
}

/// Resolves any inbound RPC method name to its canonical form.
///
/// Two steps, in order:
/// 1. the pre-rebrand `openhuman.` prefix is rewritten to `neppy.`
///    ([`normalize_rpc_method`]), so `openhuman.foo` and `neppy.foo` are the
///    same method everywhere;
/// 2. the result is looked up in the legacy alias table (whose keys and values
///    are written in the canonical `neppy.` spelling) so historical misspellings
///    such as `neppy.get_config` or the bare `health_snapshot` resolve to the
///    registered controller.
///
/// Unknown names pass through unchanged (after step 1). The function is
/// idempotent: calling it on an already-canonical name is a no-op.
pub fn resolve_legacy(method: &str) -> std::borrow::Cow<'_, str> {
    let normalized = normalize_rpc_method(method);
    for (legacy, canonical) in legacy_aliases() {
        if *legacy == normalized.as_ref() {
            return std::borrow::Cow::Borrowed(canonical);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::PathBuf;

    fn frontend_rpc_catalog_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("app/src/services/rpcMethods.ts")
    }

    fn read_frontend_rpc_catalog() -> String {
        let path = frontend_rpc_catalog_path();
        fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
    }

    fn object_body_after_marker<'a>(source: &'a str, marker: &str, terminator: &str) -> &'a str {
        let marker_start = source
            .find(marker)
            .unwrap_or_else(|| panic!("missing marker `{marker}` in frontend RPC catalog"));
        let object_start = marker_start
            + source[marker_start..]
                .find('{')
                .unwrap_or_else(|| panic!("missing object start after `{marker}`"))
            + 1;
        let rest = &source[object_start..];
        let object_end = rest
            .find(terminator)
            .unwrap_or_else(|| panic!("missing terminator `{terminator}` after `{marker}`"));
        &rest[..object_end]
    }

    fn quoted_value(text: &str) -> String {
        let (quote_index, quote) = text
            .char_indices()
            .find(|(_, ch)| *ch == '\'' || *ch == '"')
            .unwrap_or_else(|| panic!("expected quoted value in `{text}`"));
        let value_start = quote_index + quote.len_utf8();
        let rest = &text[value_start..];
        let value_end = rest
            .find(quote)
            .unwrap_or_else(|| panic!("unterminated quoted value in `{text}`"));
        rest[..value_end].to_string()
    }

    fn parse_core_rpc_methods(source: &str) -> BTreeMap<String, String> {
        let body = object_body_after_marker(source, "export const CORE_RPC_METHODS", "} as const;");
        let mut methods = BTreeMap::new();
        for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if line.starts_with("//") {
                continue;
            }
            let (key, value) = line
                .split_once(':')
                .unwrap_or_else(|| panic!("malformed CORE_RPC_METHODS entry: `{line}`"));
            methods.insert(key.trim().to_string(), quoted_value(value));
        }
        methods
    }

    fn parse_frontend_legacy_aliases(
        source: &str,
        core_methods: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let body = object_body_after_marker(source, "export const LEGACY_METHOD_ALIASES", "};");
        let compact = body
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("//"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut aliases = BTreeMap::new();
        for entry in compact
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
        {
            let (legacy, target_expr) = entry
                .split_once(':')
                .unwrap_or_else(|| panic!("expected legacy alias entry, got `{entry}`"));
            // Prettier strips quotes from keys that are valid JS identifiers
            // (e.g. `health_snapshot`), so accept both `'foo':` and bare `foo:`.
            let legacy_trimmed = legacy.trim();
            let legacy = if legacy_trimmed.starts_with('\'') || legacy_trimmed.starts_with('"') {
                quoted_value(legacy)
            } else {
                legacy_trimmed.to_string()
            };
            let target_expr = target_expr.trim();
            let canonical = if let Some(key) = target_expr.strip_prefix("CORE_RPC_METHODS.") {
                core_methods
                    .get(key)
                    .unwrap_or_else(|| {
                        panic!("legacy alias references unknown CORE_RPC_METHODS.{key}")
                    })
                    .clone()
            } else {
                quoted_value(target_expr)
            };
            aliases.insert(legacy, canonical);
        }
        aliases
    }

    fn registered_http_methods() -> BTreeSet<String> {
        crate::core::all::all_http_method_schemas()
            .into_iter()
            .map(|method| method.method)
            .collect()
    }

    /// Whether `method`'s controller is compiled out of THIS build by a
    /// default-ON Cargo feature gate.
    ///
    /// The frontend RPC catalog and the alias table below are both **data**:
    /// they are authored against the full (shipped desktop) surface and cannot
    /// be `#[cfg]`'d per Rust feature — the frontend is built independently of
    /// the core's feature set, and the shipped app always enables `mcp`. So in
    /// a slim build they legitimately still name methods whose controllers no
    /// longer exist. The drift checks below must therefore ignore exactly those
    /// namespaces, and keep asserting on everything else — otherwise the whole
    /// drift signal is lost in slim builds (or, worse, someone "fixes" the
    /// failure by deleting live frontend methods from the catalog).
    ///
    /// Mirrors how the agent loader tolerates the orchestrator TOML's dangling
    /// `mcp_agent` subagent id (#4799). Composed from one predicate per gate so
    /// each new gate adds a self-contained pair (keeps the attribute-`#[cfg]`
    /// form the feature-gate smoke lane's coverage guard tracks).
    fn is_compiled_out_method(method: &str) -> bool {
        mcp_method_compiled_out(method) || channels_method_compiled_out(method)
    }

    #[cfg(feature = "mcp")]
    fn mcp_method_compiled_out(_method: &str) -> bool {
        false
    }

    #[cfg(not(feature = "mcp"))]
    fn mcp_method_compiled_out(method: &str) -> bool {
        // `mcp` feature OFF ⇒ the `mcp_clients` (dynamic registry) and
        // `mcp_audit` (write log) controllers are unregistered.
        method.starts_with("neppy.mcp_clients_") || method.starts_with("neppy.mcp_audit_")
    }

    #[cfg(feature = "channels")]
    fn channels_method_compiled_out(_method: &str) -> bool {
        false
    }

    #[cfg(not(feature = "channels"))]
    fn channels_method_compiled_out(method: &str) -> bool {
        // `channels` feature OFF ⇒ the channels + webview_apis +
        // webview_notifications + whatsapp_data controllers are unregistered
        // (#4801). NOTE: the in-app web chat (`neppy.channel_*`) is NOT
        // gated (core product surface, #5002) — do not add that prefix here.
        method.starts_with("neppy.channels_")
            || method.starts_with("neppy.webview_apis_")
            || method.starts_with("neppy.webview_notifications_")
            || method.starts_with("neppy.whatsapp_data_")
    }

    #[test]
    fn quoted_value_extracts_single_quoted_string() {
        assert_eq!(quoted_value(": 'hello'"), "hello");
    }

    #[test]
    fn quoted_value_extracts_double_quoted_string() {
        assert_eq!(quoted_value(": \"hello\""), "hello");
    }

    #[test]
    #[should_panic(expected = "expected quoted value")]
    fn quoted_value_panics_on_unquoted_text() {
        let _ = quoted_value(": bare-token");
    }

    #[test]
    #[should_panic(expected = "unterminated quoted value")]
    fn quoted_value_panics_on_unterminated_quote() {
        let _ = quoted_value(": 'open-but-never-closed");
    }

    #[test]
    fn object_body_after_marker_returns_inner_body() {
        let source = "noise\nexport const FOO = {\n  alpha: 'a',\n  beta: 'b',\n} as const;\nrest";
        let body = object_body_after_marker(source, "export const FOO", "} as const;");
        assert!(body.contains("alpha: 'a'"));
        assert!(body.contains("beta: 'b'"));
        assert!(!body.contains("rest"));
        assert!(!body.contains("noise"));
    }

    #[test]
    #[should_panic(expected = "missing marker")]
    fn object_body_after_marker_panics_when_marker_absent() {
        let _ = object_body_after_marker("nothing here", "export const MISSING", "};");
    }

    #[test]
    #[should_panic(expected = "missing terminator")]
    fn object_body_after_marker_panics_when_terminator_absent() {
        let _ = object_body_after_marker(
            "export const FOO = { alpha: 'a',",
            "export const FOO",
            "} as const;",
        );
    }

    #[test]
    fn parse_core_rpc_methods_extracts_entries_and_skips_comments() {
        let source = "export const CORE_RPC_METHODS = {\n  // a comment that should be skipped\n  alphaMethod: 'neppy.alpha',\n  betaMethod: 'neppy.beta',\n} as const;\n";
        let methods = parse_core_rpc_methods(source);
        assert_eq!(
            methods.get("alphaMethod").map(String::as_str),
            Some("neppy.alpha")
        );
        assert_eq!(
            methods.get("betaMethod").map(String::as_str),
            Some("neppy.beta")
        );
        assert_eq!(methods.len(), 2);
    }

    #[test]
    #[should_panic(expected = "malformed CORE_RPC_METHODS entry")]
    fn parse_core_rpc_methods_panics_on_non_colon_line() {
        let source =
            "export const CORE_RPC_METHODS = {\n  alphaMethod 'neppy.alpha',\n} as const;\n";
        let _ = parse_core_rpc_methods(source);
    }

    #[test]
    fn parse_frontend_legacy_aliases_resolves_core_method_refs_and_literals() {
        let source = "export const CORE_RPC_METHODS = {\n  alphaMethod: 'neppy.alpha',\n} as const;\n\nexport const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {\n  'neppy.legacy_alpha': CORE_RPC_METHODS.alphaMethod,\n  'neppy.legacy_literal': 'neppy.literal_target',\n};\n";
        let core_methods = parse_core_rpc_methods(source);
        let aliases = parse_frontend_legacy_aliases(source, &core_methods);
        assert_eq!(
            aliases.get("neppy.legacy_alpha").map(String::as_str),
            Some("neppy.alpha")
        );
        assert_eq!(
            aliases.get("neppy.legacy_literal").map(String::as_str),
            Some("neppy.literal_target")
        );
    }

    #[test]
    fn parse_frontend_legacy_aliases_accepts_bare_identifier_keys_and_skips_comments() {
        // Prettier strips redundant quotes from keys that are valid JS
        // identifiers, so the canonical form for a simple key like
        // `health_snapshot` is unquoted. The parser must accept both
        // `'foo':` and bare `foo:`, and must ignore `//` comment lines
        // in the LEGACY_METHOD_ALIASES body.
        let source = "export const CORE_RPC_METHODS = {\n  alphaMethod: 'neppy.alpha',\n  betaMethod: 'neppy.beta',\n} as const;\n\nexport const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {\n  // legacy aliases for the alpha method\n  'neppy.legacy_alpha': CORE_RPC_METHODS.alphaMethod,\n  beta_legacy: CORE_RPC_METHODS.betaMethod,\n};\n";
        let core_methods = parse_core_rpc_methods(source);
        let aliases = parse_frontend_legacy_aliases(source, &core_methods);
        assert_eq!(
            aliases.get("neppy.legacy_alpha").map(String::as_str),
            Some("neppy.alpha"),
            "quoted-key entry should still resolve"
        );
        assert_eq!(
            aliases.get("beta_legacy").map(String::as_str),
            Some("neppy.beta"),
            "bare-identifier key should resolve (Prettier-normalized form)"
        );
        assert!(
            !aliases
                .keys()
                .any(|k| k.contains("//") || k.contains("legacy aliases")),
            "comment text must not be captured as a key"
        );
    }

    #[test]
    #[should_panic(expected = "legacy alias references unknown CORE_RPC_METHODS")]
    fn parse_frontend_legacy_aliases_panics_on_unknown_core_method_ref() {
        let source = "export const CORE_RPC_METHODS = {\n  alphaMethod: 'neppy.alpha',\n} as const;\n\nexport const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {\n  'neppy.legacy_alpha': CORE_RPC_METHODS.doesNotExist,\n};\n";
        let core_methods = parse_core_rpc_methods(source);
        let _ = parse_frontend_legacy_aliases(source, &core_methods);
    }

    #[test]
    fn resolve_legacy_rewrites_every_table_entry() {
        for (legacy, canonical) in LEGACY_ALIASES {
            assert_eq!(
                resolve_legacy(legacy),
                *canonical,
                "expected legacy alias {legacy} to resolve to {canonical}",
            );
        }
    }

    #[test]
    fn normalize_rpc_method_rewrites_only_the_legacy_prefix() {
        assert_eq!(
            normalize_rpc_method("openhuman.memory_doc_put"),
            "neppy.memory_doc_put"
        );
        // Already canonical / unrelated names are borrowed, unchanged.
        for m in [
            "neppy.memory_doc_put",
            "core.ping",
            "channels.list",
            "",
            "openhumanx.foo",
            "x.openhuman.foo",
            "OPENHUMAN.foo",
        ] {
            assert!(
                matches!(normalize_rpc_method(m), std::borrow::Cow::Borrowed(_)),
                "{m} must pass through untouched"
            );
            assert_eq!(normalize_rpc_method(m), m);
        }
    }

    #[test]
    fn resolve_legacy_accepts_the_old_prefix_for_every_table_entry() {
        // The table is written in the canonical `neppy.` spelling; the old
        // `openhuman.` spelling of every prefixed key must resolve identically.
        for (legacy, canonical) in LEGACY_ALIASES {
            if let Some(rest) = legacy.strip_prefix(RPC_METHOD_PREFIX) {
                let old = format!("{LEGACY_RPC_METHOD_PREFIX}{rest}");
                assert_eq!(
                    resolve_legacy(&old),
                    *canonical,
                    "old spelling {old} must resolve to {canonical}"
                );
            }
        }
        assert_eq!(resolve_legacy("openhuman.ping"), "core.ping");
        assert_eq!(resolve_legacy("neppy.ping"), "core.ping");
        assert_eq!(resolve_legacy("openhuman.get_config"), "neppy.config_get");
    }

    #[test]
    fn resolve_legacy_maps_old_prefix_passthrough_to_neppy() {
        assert_eq!(
            resolve_legacy("openhuman.memory_list_namespaces"),
            "neppy.memory_list_namespaces"
        );
        assert_eq!(
            resolve_legacy("neppy.memory_list_namespaces"),
            "neppy.memory_list_namespaces"
        );
    }

    #[test]
    fn resolve_legacy_rewrites_composio_trigger_settings() {
        // The specific case observed in Sentry: older bundles called the
        // bare `neppy.update_composio_trigger_settings` against a core
        // that only registers the namespaced form.
        assert_eq!(
            resolve_legacy("openhuman.update_composio_trigger_settings"),
            "neppy.config_update_composio_trigger_settings",
        );
        assert_eq!(
            resolve_legacy("neppy.update_composio_trigger_settings"),
            "neppy.config_update_composio_trigger_settings",
        );
    }

    #[test]
    fn resolve_legacy_rewrites_bare_health_snapshot() {
        // Sentry CORE-RUST-FG: older clients (and some SDK callers) issued
        // `health_snapshot` without the `openhuman.` namespace prefix.  The
        // alias table must rewrite it to the canonical form so the call
        // resolves against the registered controller.
        assert_eq!(resolve_legacy("health_snapshot"), "neppy.health_snapshot",);
    }

    #[test]
    fn resolve_legacy_rewrites_health_probe_variants() {
        // #3566 / Sentry CORE-2C: older clients and SDK callers issued the
        // health snapshot under several legacy spellings (bare `health`, and
        // the dotted `health.snapshot` / `health.status` / `health.get`).
        // There is no distinct status/get handler, so every variant must
        // resolve to the canonical `neppy.health_snapshot`.
        for legacy in ["health", "health.get", "health.snapshot", "health.status"] {
            assert_eq!(
                resolve_legacy(legacy),
                "neppy.health_snapshot",
                "expected health probe variant {legacy} to resolve to the snapshot method",
            );
        }
    }

    #[test]
    fn resolve_legacy_bare_health_snapshot_regression() {
        // Sentry CORE-2C regression guard: bare `health_snapshot` (no
        // namespace prefix) must keep resolving to the canonical method. The
        // CORE-2C events were stale (release 0.53.43 predated the alias added
        // in #2853), but this lock-in proves the alias still fires on the
        // exact-match resolver so the bare form can never regress.
        assert_eq!(resolve_legacy("health_snapshot"), "neppy.health_snapshot",);
    }

    #[test]
    fn resolve_legacy_rewrites_system_info() {
        // Sentry CORE-RUST-G0: older clients called `neppy.system_info`
        // before the method was namespaced under `health` as
        // `neppy.health_system_info`.  The alias table must rewrite it so
        // the call resolves against the registered controller.
        assert_eq!(
            resolve_legacy("neppy.system_info"),
            "neppy.health_system_info",
        );
    }

    #[test]
    fn resolve_legacy_passes_through_unknown_methods() {
        assert_eq!(
            resolve_legacy("neppy.memory_list_namespaces"),
            "neppy.memory_list_namespaces"
        );
        assert_eq!(resolve_legacy("does.not.exist"), "does.not.exist");
        assert_eq!(resolve_legacy(""), "");
    }

    #[test]
    fn resolve_legacy_is_idempotent_for_canonical_names() {
        // Canonical names already match what the registry expects;
        // running them through the resolver must be a no-op so callers
        // can wrap the lookup unconditionally.
        for (_, canonical) in LEGACY_ALIASES {
            assert_eq!(
                resolve_legacy(canonical),
                *canonical,
                "canonical {canonical} must pass through unchanged",
            );
        }
    }

    #[test]
    fn resolve_legacy_returned_str_equals_table_value() {
        // Sanity check: the function returns the canonical str slice from
        // the table when it matches, not a copy of the input.
        let out = resolve_legacy("neppy.ping");
        assert_eq!(out, "core.ping");
    }

    #[test]
    fn resolve_legacy_rewrites_dotted_channel_list_aliases() {
        assert_eq!(resolve_legacy("channels.list"), "neppy.channels_list");
        assert_eq!(resolve_legacy("neppy.channels.list"), "neppy.channels_list");
    }

    #[test]
    fn resolve_legacy_rewrites_tool_registry_diagnostics() {
        // #3294: the dotted `tool_registry.diagnostics` spelling sent by older
        // desktop bundles must resolve to the canonical controller name so the
        // Tool Policy diagnostics panel works instead of failing with
        // "unknown method".
        assert_eq!(
            resolve_legacy("tool_registry.diagnostics"),
            "neppy.tool_registry_diagnostics"
        );
    }

    #[test]
    fn frontend_core_rpc_methods_exist_in_core_schema_registry() {
        let source = read_frontend_rpc_catalog();
        let core_methods = parse_core_rpc_methods(&source);
        let registered = registered_http_methods();
        let missing: Vec<_> = core_methods
            .values()
            .filter(|method| !registered.contains(*method))
            .filter(|method| !is_compiled_out_method(method))
            .cloned()
            .collect();

        assert!(
            missing.is_empty(),
            "frontend CORE_RPC_METHODS contains methods absent from all_http_method_schemas(): {missing:?}"
        );
    }

    #[test]
    fn frontend_legacy_aliases_match_server_alias_table() {
        let source = read_frontend_rpc_catalog();
        let core_methods = parse_core_rpc_methods(&source);
        let frontend_aliases = parse_frontend_legacy_aliases(&source, &core_methods);
        let server_aliases: BTreeMap<String, String> = legacy_aliases()
            .iter()
            .map(|(legacy, canonical)| ((*legacy).to_string(), (*canonical).to_string()))
            .collect();

        assert_eq!(
            frontend_aliases, server_aliases,
            "frontend LEGACY_METHOD_ALIASES must stay in sync with src/core/legacy_aliases.rs"
        );
    }

    #[test]
    fn legacy_alias_targets_exist_in_core_schema_registry() {
        let registered = registered_http_methods();
        let missing: Vec<_> = legacy_aliases()
            .iter()
            .filter(|(_, canonical)| !registered.contains(*canonical))
            .filter(|(_, canonical)| !is_compiled_out_method(canonical))
            .map(|(legacy, canonical)| format!("{legacy} -> {canonical}"))
            .collect();

        assert!(
            missing.is_empty(),
            "legacy aliases point at methods absent from all_http_method_schemas(): {missing:?}"
        );
    }
}
