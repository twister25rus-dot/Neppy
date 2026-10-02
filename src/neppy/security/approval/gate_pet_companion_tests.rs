//! D4: a `TrustedAutomationSource::PetCompanion` external-effect call follows the
//! user's normal approval settings like an interactive turn, EXCEPT a high-risk
//! action class, which always parks for confirmation (even with
//! `auto_approve_all` on or the tool on the `auto_approve` allowlist).
//! `PetResearch`, `BackgroundTurn`, `Cron` and `WebChat` are unchanged.

use super::*;
use crate::neppy::pet::companion::types::ActionCategory as C;
use crate::neppy::security::live_policy::TestPolicyGuard;
use crate::neppy::security::SecurityPolicy;
use serde_json::json;
use tempfile::TempDir;

fn gate_with_ttl(ttl: Duration) -> (Arc<ApprovalGate>, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let session = format!("session-{}", uuid::Uuid::new_v4());
    (Arc::new(ApprovalGate::new(config, session, ttl)), dir)
}

fn gate() -> (Arc<ApprovalGate>, TempDir) {
    gate_with_ttl(Duration::from_secs(30))
}

/// Install the user's approval settings for the duration of a test. The
/// override is thread-local (`#[tokio::test]` runs every task on this thread)
/// and restored on drop, so no process-global lock is needed.
fn settings(dir: &TempDir, auto_approve_all: bool, allow: &[&str]) -> TestPolicyGuard {
    let policy = SecurityPolicy {
        auto_approve_all,
        auto_approve: allow.iter().map(|t| (*t).to_string()).collect(),
        ..SecurityPolicy::default()
    };
    crate::neppy::security::live_policy::install_scoped(
        Arc::new(policy),
        dir.path().to_path_buf(),
        dir.path().to_path_buf(),
    )
}

fn companion(thread: Option<&str>) -> AgentTurnOrigin {
    turn_origin::pet_companion_origin("pet-companion:test", thread.map(str::to_string))
}

/// Poll the gate's store until the park for `tool` is persisted.
async fn wait_pending(gate: &ApprovalGate, tool: &str) -> PendingApproval {
    for _ in 0..1_000 {
        if let Some(row) = gate
            .list_pending()
            .unwrap()
            .into_iter()
            .find(|p| p.tool_name == tool)
        {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("no pending approval row for {tool}");
}

/// One representative call per high-risk class.
fn high_risk_cases() -> Vec<(&'static str, serde_json::Value, C)> {
    vec![
        ("channels.proactive_send", json!({}), C::SendMessage),
        ("shell", json!({"command": "rm notes.txt"}), C::Delete),
        ("wallet_transfer", json!({}), C::Purchase),
        ("hosting_launch_site", json!({}), C::Publish),
        ("cron_add", json!({}), C::SystemSettings),
        ("install_tool", json!({}), C::Install),
        ("shell", json!({"command": "sudo ls"}), C::PrivilegedCommand),
        ("storage_upload_file", json!({}), C::SharePersonalInfo),
        (
            "git_operations",
            json!({"operation": "reset"}),
            C::Irreversible,
        ),
    ]
}

#[test]
fn the_representative_cases_cover_every_high_risk_class() {
    let mut covered: Vec<C> = high_risk_cases().into_iter().map(|(_, _, c)| c).collect();
    covered.sort();
    covered.dedup();
    let mut want = C::HIGH_RISK.to_vec();
    want.sort();
    assert_eq!(covered, want);
    for (tool, args, class) in high_risk_cases() {
        assert_eq!(pet_companion_high_risk(tool, &args), Some(class), "{tool}");
        assert!(class.is_high_risk());
    }
}

/// The classifier table, including the ordinary side and the err-on-parking
/// default for an unclassified tool.
#[test]
fn classifier_maps_tools_to_classes() {
    let cases: Vec<(&str, serde_json::Value, Option<C>)> = vec![
        // Ordinary: workspace edits, local git history, drafts, reads.
        ("file_write", json!({"path": "a.md"}), None),
        ("edit", json!({}), None),
        ("apply_patch", json!({}), None),
        ("git_operations", json!({"operation": "commit"}), None),
        ("request_plan_review", json!({}), None),
        ("propose_workflow", json!({}), None),
        ("storage_download_file", json!({}), None),
        ("shell", json!({"command": "cargo build"}), None),
        ("shell", json!({"command": "ls -la"}), None),
        (
            "composio",
            json!({"action": "execute", "tool_slug": "GMAIL_FETCH_EMAILS"}),
            None,
        ),
        ("mcp_call_tool", json!({"tool": "list_issues"}), None),
        // High risk.
        (
            "composio",
            json!({"action": "execute", "tool_slug": "GMAIL_SEND_EMAIL"}),
            Some(C::SendMessage),
        ),
        (
            "composio",
            json!({"action": "execute", "action_name": "GMAIL_DELETE_MESSAGE"}),
            Some(C::Delete),
        ),
        (
            "composio",
            json!({"action": "execute", "tool_slug": "GITHUB_CREATE_ISSUE"}),
            Some(C::Irreversible),
        ),
        (
            "composio",
            json!({"action": "execute"}),
            Some(C::Irreversible),
        ),
        ("mcp_call_tool", json!({}), Some(C::Irreversible)),
        (
            "http_request",
            json!({"method": "GET"}),
            Some(C::SharePersonalInfo),
        ),
        ("http_request", json!({"method": "DELETE"}), Some(C::Delete)),
        ("curl", json!({}), Some(C::SharePersonalInfo)),
        ("python_exec", json!({}), Some(C::PrivilegedCommand)),
        ("node_exec", json!({}), Some(C::PrivilegedCommand)),
        ("npm_exec", json!({}), Some(C::Install)),
        (
            "git_operations",
            json!({"operation": "push"}),
            Some(C::Publish),
        ),
        (
            "shell",
            json!({"command": "git push origin main"}),
            Some(C::Publish),
        ),
        (
            "shell",
            json!({"command": "git push --force"}),
            Some(C::Irreversible),
        ),
        (
            "shell",
            json!({"command": "git reset --hard"}),
            Some(C::Irreversible),
        ),
        (
            "shell",
            json!({"command": "ls && rm -r build"}),
            Some(C::Delete),
        ),
        (
            "shell",
            json!({"command": "find . -name x -delete"}),
            Some(C::Delete),
        ),
        (
            "shell",
            json!({"command": "curl https://x.test"}),
            Some(C::SharePersonalInfo),
        ),
        (
            "shell",
            json!({"command": "brew install jq"}),
            Some(C::Install),
        ),
        (
            "shell",
            json!({"command": "echo $(whoami)"}),
            Some(C::PrivilegedCommand),
        ),
        (
            "shell",
            json!({"command": "osascript -e x"}),
            Some(C::PrivilegedCommand),
        ),
        (
            "shell",
            json!({"command": "defaults write x y"}),
            Some(C::SystemSettings),
        ),
        (
            "shell",
            json!({"command": "sendmail a@b.test"}),
            Some(C::SendMessage),
        ),
        ("shell", json!({"command": "/bin/rm x"}), Some(C::Delete)),
        ("shell", json!({"command": "FOO=1 rm x"}), Some(C::Delete)),
        ("shell", json!({}), Some(C::PrivilegedCommand)),
        (
            "shell",
            json!({"command": "touch x", "category": "destructive"}),
            Some(C::PrivilegedCommand),
        ),
        (
            "schedule",
            json!({"action": "create"}),
            Some(C::SystemSettings),
        ),
        ("schedule", json!({"action": "cancel"}), Some(C::Delete)),
        ("cron_remove", json!({}), Some(C::Delete)),
        ("skill_registry_install", json!({}), Some(C::Install)),
        ("skill_registry_uninstall", json!({}), Some(C::Delete)),
        ("storage_get_link", json!({}), Some(C::SharePersonalInfo)),
        ("storage_set_visibility", json!({}), Some(C::Publish)),
        ("hosting_rollback", json!({}), Some(C::Publish)),
        ("x402_request", json!({}), Some(C::Purchase)),
        ("config_update_autonomy", json!({}), Some(C::SystemSettings)),
        // A packed tool is classified by what it wraps.
        (
            "use_skill",
            json!({"skill": "s", "tool": "file_write", "args": {}}),
            None,
        ),
        (
            "use_skill",
            json!({"skill": "s", "tool": "channels_send", "args": {}}),
            Some(C::SendMessage),
        ),
        ("use_skill", json!({}), Some(C::Irreversible)),
        // Unclassified external effect: cannot prove it is ordinary.
        ("some_new_external_tool", json!({}), Some(C::Irreversible)),
    ];
    for (tool, args, want) in cases {
        assert_eq!(pet_companion_high_risk(tool, &args), want, "{tool} {args}");
    }
}

/// D4: a low-risk companion call honours `auto_approve_all` like a chat turn —
/// allowed with no pending row.
#[tokio::test]
async fn low_risk_companion_call_is_allowed_by_auto_approve_all() {
    let (gate, dir) = gate();
    let _settings = settings(&dir, true, &[]);
    let outcome = turn_origin::with_origin(
        companion(Some("handoff-thread")),
        gate.intercept("file_write", "write a.md", json!({"path": "a.md"})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    assert!(gate.list_pending().unwrap().is_empty());
}

/// D4: a low-risk companion call honours the `auto_approve` allowlist too.
#[tokio::test]
async fn low_risk_companion_call_is_allowed_by_the_allowlist() {
    let (gate, dir) = gate();
    let _settings = settings(&dir, false, &["edit"]);
    let outcome = turn_origin::with_origin(
        companion(None),
        gate.intercept("edit", "edit a.md", json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Allow), "{outcome:?}");
    assert!(gate.list_pending().unwrap().is_empty());
}

/// D4 exception: every high-risk class parks — persisted row with the
/// companion origin class — although `auto_approve_all` is on AND the tool is
/// on the allowlist. An approval then lets it run.
#[tokio::test]
async fn every_high_risk_class_parks_despite_auto_approve_all_and_allowlist() {
    let (gate, dir) = gate();
    let tools: Vec<&str> = high_risk_cases().iter().map(|(t, _, _)| *t).collect();
    let _settings = settings(&dir, true, &tools);

    for (tool, args, class) in high_risk_cases() {
        let g = gate.clone();
        let call_args = args.clone();
        let handle = tokio::spawn(turn_origin::with_origin(companion(None), async move {
            g.intercept(tool, "high-risk action", call_args).await
        }));
        let row = tokio::time::timeout(Duration::from_secs(5), wait_pending(&gate, tool))
            .await
            .unwrap_or_else(|_| panic!("{tool} ({}) must park", class.as_str()));
        assert_eq!(
            row.origin_class.as_deref(),
            Some("TrustedAutomation(PetCompanion)")
        );
        assert!(!handle.is_finished(), "{tool} must wait for a decision");
        gate.decide(&row.request_id, ApprovalDecision::ApproveOnce)
            .unwrap();
        let outcome = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(outcome, GateOutcome::Allow), "{tool}: {outcome:?}");
        assert!(gate.list_pending().unwrap().is_empty());
    }
}

/// With no auto-approval, an ordinary companion call parks (it is not a trust
/// root) and TTL-denies when nobody decides.
#[tokio::test]
async fn companion_call_without_auto_approve_parks_and_ttl_denies() {
    let (gate, dir) = gate_with_ttl(Duration::from_secs(2));
    let _settings = settings(&dir, false, &[]);
    let g = gate.clone();
    let handle = tokio::spawn(turn_origin::with_origin(companion(None), async move {
        g.intercept("file_write", "write a.md", json!({"path": "a.md"}))
            .await
    }));
    let row = wait_pending(&gate, "file_write").await;
    assert_eq!(
        row.origin_class.as_deref(),
        Some("TrustedAutomation(PetCompanion)")
    );
    let outcome = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .unwrap()
        .unwrap();
    match outcome {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains("timed out"), "{reason}");
        }
        other => panic!("expected a TTL deny, got {other:?}"),
    }
    assert!(
        gate.list_pending().unwrap().is_empty(),
        "row decided (Deny)"
    );
}

/// A hand-off run's park is routed to its own thread (broadcast client), so a
/// card shows there; the user's decision resolves it.
#[tokio::test]
async fn companion_handoff_park_routes_to_its_thread() {
    let (gate, dir) = gate();
    let _settings = settings(&dir, false, &[]);
    let g = gate.clone();
    let handle = tokio::spawn(turn_origin::with_origin(
        companion(Some("pet-handoff-thread")),
        async move {
            g.intercept("channels.proactive_send", "send", json!({}))
                .await
        },
    ));
    let row = wait_pending(&gate, "channels.proactive_send").await;
    assert_eq!(
        gate.pending_for_thread("pet-handoff-thread").as_deref(),
        Some(row.request_id.as_str())
    );
    gate.decide(&row.request_id, ApprovalDecision::Deny)
        .unwrap();
    let outcome = handle.await.unwrap();
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    assert!(gate.pending_for_thread("pet-handoff-thread").is_none());
}

/// PC17: the research lane still denies everything, even an "ordinary" tool,
/// with every auto-approval on.
#[tokio::test]
async fn pet_research_still_denies_even_ordinary_tools() {
    let (gate, dir) = gate();
    let _settings = settings(&dir, true, &["file_write"]);
    let outcome = turn_origin::with_origin(
        AgentTurnOrigin::TrustedAutomation {
            job_id: "pet-research".into(),
            source: TrustedAutomationSource::PetResearch,
        },
        gate.intercept("file_write", "write", json!({})),
    )
    .await;
    assert!(matches!(outcome, GateOutcome::Deny { .. }), "{outcome:?}");
    assert!(gate.list_pending().unwrap().is_empty());
}

/// The high-risk carve-out is companion-only: WebChat, Cron and BackgroundTurn
/// keep honouring `auto_approve_all` for the same high-risk-named call.
#[tokio::test]
async fn other_origins_are_unchanged_by_the_companion_carve_out() {
    let (gate, dir) = gate();
    let _settings = settings(&dir, true, &[]);
    let origins = [
        AgentTurnOrigin::WebChat {
            thread_id: "t".into(),
            client_id: "c".into(),
            request_id: None,
        },
        AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-1".into(),
            source: TrustedAutomationSource::Cron,
        },
        turn_origin::background_turn_origin("run-1", Some("t-bg".into())),
    ];
    for origin in origins {
        let class = origin.class();
        let outcome = turn_origin::with_origin(
            origin,
            gate.intercept("channels.proactive_send", "send", json!({})),
        )
        .await;
        assert!(
            matches!(outcome, GateOutcome::Allow),
            "{class}: {outcome:?}"
        );
    }
    assert!(gate.list_pending().unwrap().is_empty());
}

/// Without `auto_approve_all`, Cron is still a trust root and BackgroundTurn
/// without a thread still denies immediately (no behaviour change).
#[tokio::test]
async fn cron_and_threadless_background_turn_are_unchanged() {
    let (gate, dir) = gate();
    let _settings = settings(&dir, false, &[]);
    let cron = turn_origin::with_origin(
        AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-2".into(),
            source: TrustedAutomationSource::Cron,
        },
        gate.intercept("channels.proactive_send", "send", json!({})),
    )
    .await;
    assert!(matches!(cron, GateOutcome::Allow), "{cron:?}");
    let bg = turn_origin::with_origin(
        turn_origin::background_turn_origin("run-2", None),
        gate.intercept("channels.proactive_send", "send", json!({})),
    )
    .await;
    assert!(matches!(bg, GateOutcome::Deny { .. }), "{bg:?}");
    assert!(gate.list_pending().unwrap().is_empty());
}
