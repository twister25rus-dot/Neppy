//! Tests for local-only egress enforcement (privacy epic S7, #4441).
//!
//! The pure [`local_only_blocks`] / [`is_control_plane`] truth tables need no
//! process-global state. The two side-effecting wrappers ([`enforce_egress`],
//! [`local_only_tool_block`]) read the live policy, so those tests install a
//! LocalOnly / Standard policy under [`TEST_ENV_LOCK`] (shared with the other
//! `live_policy`-touching tests so installs never race) and restore Standard on
//! the way out.

use super::*;
use crate::openhuman::config::PrivacyMode;
use crate::openhuman::security::egress::{DataKind, EgressDescriptor, EgressReason};

// ── Pure decision: local_only_blocks truth table ──────────────────────────

fn external_composio() -> EgressDescriptor {
    EgressDescriptor::composio("GITHUB_CREATE_ISSUE")
}

#[test]
fn local_only_blocks_external_transfer() {
    // LocalOnly + external user-data transfer → blocked.
    assert!(local_only_blocks(
        PrivacyMode::LocalOnly,
        &external_composio()
    ));
}

#[test]
fn local_only_blocks_each_external_surface() {
    // AC7: the block applies across every egress surface's user-data descriptor.
    let surfaces = [
        EgressDescriptor::inference("openai", "gpt-4o", true),
        EgressDescriptor::composio("SLACK_SEND_MESSAGE"),
        EgressDescriptor::embedding("cloud", "text-embedding-3-small"),
        EgressDescriptor::network_fetch("api.example.com"),
        EgressDescriptor::integration("/agent-integrations/parallel/research"),
    ];
    for desc in &surfaces {
        assert!(
            local_only_blocks(PrivacyMode::LocalOnly, desc),
            "surface {:?}/{} must block under LocalOnly",
            desc.reason,
            desc.service
        );
        assert!(
            !local_only_blocks(PrivacyMode::Standard, desc),
            "surface {:?}/{} must be allowed under Standard",
            desc.reason,
            desc.service
        );
    }
}

#[test]
fn standard_mode_allows_everything() {
    // Standard / Sensitive never block here.
    assert!(!local_only_blocks(
        PrivacyMode::Standard,
        &external_composio()
    ));
    assert!(!local_only_blocks(
        PrivacyMode::Sensitive,
        &external_composio()
    ));
}

#[test]
fn local_only_allows_local_runtime() {
    // A non-external transfer (local runtime — Ollama/LM Studio/etc.) is never
    // blocked: nothing leaves the device.
    let local = EgressDescriptor::inference("ollama", "llama3", false);
    assert!(!local.is_external);
    assert!(!local_only_blocks(PrivacyMode::LocalOnly, &local));
}

#[test]
fn local_only_allows_control_plane_integration() {
    // LocalOnly + an exempt control-plane integration path → allowed.
    let pricing = EgressDescriptor::integration("/agent-integrations/pricing");
    assert!(!local_only_blocks(PrivacyMode::LocalOnly, &pricing));
}

#[test]
fn local_only_blocks_user_data_integration() {
    // LocalOnly + a user-data integration path → blocked.
    let execute = EgressDescriptor::integration("/agent-integrations/composio/execute");
    assert!(local_only_blocks(PrivacyMode::LocalOnly, &execute));
}

// ── Pure decision: is_control_plane boundary ──────────────────────────────

#[test]
fn control_plane_exempts_non_tool_namespace() {
    // A control-plane call re-homed onto an integration descriptor (defensive
    // path (1)): anything not under /agent-integrations/ is exempt.
    for path in [
        "/teams/me/usage",
        "/payments/stripe/currentPlan",
        "/auth/refresh",
    ] {
        assert!(
            is_control_plane(&EgressDescriptor::integration(path)),
            "{path} must be treated as control-plane"
        );
    }
}

#[test]
fn control_plane_exempts_composio_management_and_pricing() {
    // Connection-management / catalog / OAuth / pricing carry no user content —
    // the read-only allow-list the Connections UI needs under LocalOnly.
    for path in [
        "/agent-integrations/pricing",
        "/agent-integrations/composio/connections",
        // Per-connection delete rides the same `connections` head → exempt.
        "/agent-integrations/composio/connections/conn_123",
        "/agent-integrations/composio/authorize",
        "/agent-integrations/composio/tools",
        "/agent-integrations/composio/toolkits",
    ] {
        assert!(
            is_control_plane(&EgressDescriptor::integration(path)),
            "{path} must be exempt (control-plane)"
        );
    }
}

#[test]
fn control_plane_does_not_exempt_user_data_paths() {
    // composio/execute ships tool arguments; composio/triggers[/available]
    // POST user-supplied slug/connectionId/triggerConfig (user-data writes);
    // github/repos reveals user-adjacent data; the non-composio tool namespaces
    // ship queries / content — none are exempt (fail-closed).
    for path in [
        "/agent-integrations/composio/execute",
        // Trigger writes: create_trigger / enable_trigger POST here. The
        // descriptor carries no HTTP method, so the same-path reads block too.
        "/agent-integrations/composio/triggers",
        "/agent-integrations/composio/triggers/available",
        "/agent-integrations/composio/github/repos",
        // An unknown composio sub-route defaults to blocked (fail-closed).
        "/agent-integrations/composio/some-future-write",
        "/agent-integrations/parallel/research",
        "/agent-integrations/tinyfish/fetch",
        "/agent-integrations/twilio/call",
        "/agent-integrations/file-storage/files",
        "/agent-integrations/google-places/search",
    ] {
        assert!(
            !is_control_plane(&EgressDescriptor::integration(path)),
            "{path} must NOT be exempt (ships user data)"
        );
    }
}

#[test]
fn control_plane_only_applies_to_integration_reason() {
    // A network fetch whose host happens to spell a backend path is still a
    // user-data transfer, never control-plane.
    let net = EgressDescriptor::network_fetch("agent-integrations");
    assert_eq!(net.reason, EgressReason::NetworkFetch);
    assert!(!is_control_plane(&net));
}

// ── Side-effecting wrappers (read the live policy) ────────────────────────
//
// These use the thread-scoped `test_privacy_scope` override rather than
// installing into the process-global policy, so they never race sibling tests
// that read `current_privacy_mode` on other threads (see `live_policy`).

use crate::openhuman::security::live_policy::test_privacy_scope;

#[test]
fn enforce_egress_blocks_under_local_only_and_allows_otherwise() {
    {
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        // A user-data transfer is refused with a clean, service-naming message.
        let err = enforce_egress(&external_composio()).expect_err("must block under LocalOnly");
        let msg = err.to_string();
        assert!(msg.contains("Local-only privacy mode is active"), "{msg}");
        assert!(
            msg.contains("GITHUB_CREATE_ISSUE"),
            "names the service: {msg}"
        );
        // A control-plane integration round-trip still flows.
        enforce_egress(&EgressDescriptor::integration(
            "/agent-integrations/composio/connections",
        ))
        .expect("control-plane must be allowed under LocalOnly");
    }

    // Standard mode permits the same user-data transfer.
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    enforce_egress(&external_composio()).expect("Standard mode must allow");
}

#[test]
fn local_only_tool_block_marks_message_and_clears_when_allowed() {
    let mut desc = EgressDescriptor::network_fetch("api.example.com");
    desc = desc.with_data_kind(DataKind::ToolArguments);

    {
        let _mode = test_privacy_scope(PrivacyMode::LocalOnly);
        let msg = local_only_tool_block(&desc).expect("network fetch blocked under LocalOnly");
        assert!(
            msg.starts_with(crate::openhuman::security::POLICY_BLOCKED_MARKER),
            "tool block must carry the policy-blocked marker: {msg}"
        );
        assert!(msg.contains("api.example.com"), "names the host: {msg}");
    }

    // Standard mode → no block.
    let _mode = test_privacy_scope(PrivacyMode::Standard);
    assert!(local_only_tool_block(&desc).is_none());
}

// ── Pure decision: local_mode_blocks truth table ──────────────────────────
//
// Local Mode's rule is the mirror image of `local_only_blocks`: where the
// privacy rule exempts the backend control plane, this one blocks precisely it
// and lets third-party traffic through. These tests pin both halves, because
// getting either backwards silently defeats the feature — the wrong direction
// would either keep calling the hosted backend or take a user's own vendor
// accounts down with it.

#[test]
fn local_mode_blocks_backend_round_trips() {
    for path in [
        "/agent-integrations/composio/execute",
        "/agent-integrations/composio/connections",
        "/agent-integrations/parallel/research",
        "/agent-integrations/pricing",
        "/teams/me/usage",
    ] {
        assert!(
            local_mode_blocks(true, &EgressDescriptor::integration(path)),
            "{path} is a hosted-backend round-trip and must be refused in local mode"
        );
    }
}

#[test]
fn local_mode_blocks_the_control_plane_that_privacy_mode_exempts() {
    // The whole reason this rule exists. `connections` / `authorize` / catalog
    // reads are exempt under LocalOnly (blocking sign-in buys no privacy) and
    // are exactly the calls that keep a local install tied to the backend.
    for path in [
        "/agent-integrations/composio/connections",
        "/agent-integrations/composio/authorize",
        "/agent-integrations/composio/toolkits",
        "/teams/me/usage",
    ] {
        let desc = EgressDescriptor::integration(path);
        assert!(
            !local_only_blocks(PrivacyMode::LocalOnly, &desc),
            "precondition: {path} is control-plane-exempt under LocalOnly"
        );
        assert!(
            local_mode_blocks(true, &desc),
            "{path} must nonetheless be refused in local mode"
        );
    }
}

#[test]
fn local_mode_leaves_third_party_traffic_alone() {
    // Local mode drops *our* backend, not the user's own vendor accounts. A
    // rule that blocked these would make local mode an offline switch, which is
    // what `PrivacyMode::LocalOnly` is for.
    let third_party = [
        EgressDescriptor::inference("openai", "gpt-4o", true),
        EgressDescriptor::composio("SLACK_SEND_MESSAGE"),
        EgressDescriptor::embedding("voyage", "voyage-3"),
        EgressDescriptor::network_fetch("api.example.com"),
    ];
    for desc in &third_party {
        assert!(
            !local_mode_blocks(true, desc),
            "{:?} goes to a third party, not to our backend",
            desc.reason
        );
    }
}

#[test]
fn local_mode_permits_everything_when_it_is_off() {
    assert!(!local_mode_blocks(
        false,
        &EgressDescriptor::integration("/agent-integrations/composio/execute")
    ));
}

#[test]
fn local_mode_permits_a_transfer_that_never_leaves_the_device() {
    // `is_external == false` is a local runtime; there is nothing to refuse.
    let mut desc = EgressDescriptor::integration("/agent-integrations/composio/execute");
    desc.is_external = false;
    assert!(!local_mode_blocks(true, &desc));
}

#[test]
fn local_mode_and_privacy_local_only_compose() {
    // Both on: the backend round-trip is refused by local mode, the third-party
    // call by privacy mode, and nothing leaves.
    let backend = EgressDescriptor::integration("/agent-integrations/composio/connections");
    let third_party = EgressDescriptor::inference("openai", "gpt-4o", true);

    assert!(local_mode_blocks(true, &backend));
    assert!(local_only_blocks(PrivacyMode::LocalOnly, &third_party));
}

#[test]
fn the_local_mode_message_names_the_service_and_its_alternative() {
    // Same text the local backend's 501 carries, so the user sees one
    // consistent answer wherever the call was refused.
    let message = local_mode_block_message(&EgressDescriptor::integration(
        "/agent-integrations/composio/execute",
    ));
    assert!(message.contains("/agent-integrations/composio/execute"));
    assert!(message.contains("MCP"), "must name the local alternative");
}

#[test]
fn the_local_mode_message_falls_back_for_an_unclaimed_route() {
    let message = local_mode_block_message(&EgressDescriptor::integration("/some/future/route"));
    assert!(message.contains("/some/future/route"));
    assert!(message.contains("local mode"));
}
