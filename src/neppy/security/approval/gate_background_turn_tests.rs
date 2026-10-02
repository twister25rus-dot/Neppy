//! M1: background turns (delivery follow-ups, task-board card runs) ask like an
//! interactive turn on their chat thread instead of running as a trust root.

use super::*;
use tempfile::TempDir;

fn gate() -> (Arc<ApprovalGate>, TempDir) {
    let dir = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let session = format!("session-{}", uuid::Uuid::new_v4());
    (
        Arc::new(ApprovalGate::new(config, session, Duration::from_secs(30))),
        dir,
    )
}

/// Drain the process-wide bus until the `ApprovalRequested` for `tool` arrives.
async fn find_approval_requested(
    rx: &mut tinybus::events::EventReceiver<crate::core::events::DomainEvent>,
    tool: &str,
) -> (String, Option<String>, Option<String>) {
    loop {
        match rx.recv().await {
            Some(crate::core::events::DomainEvent::ApprovalRequested {
                request_id,
                tool_name,
                thread_id,
                client_id,
                ..
            }) if tool_name == tool => return (request_id, thread_id, client_id),
            Some(_) => continue,
            None => panic!("the bus closed before the ApprovalRequested arrived"),
        }
    }
}

/// A background turn on a chat thread parks the external-effect call and
/// surfaces the card on that thread (broadcast client), with no task-local chat
/// context — the thread rides on the origin. Before M1 these turns ran as `Cli`
/// and the call was allowed without anyone being asked.
#[tokio::test]
async fn background_turn_on_a_thread_parks_for_approval_on_that_thread() {
    crate::core::bus::init().await.expect("bus init");
    let mut event_rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let (gate, _dir) = gate();
    let tool = "composio_bg_turn_park";

    let g = gate.clone();
    let handle = tokio::spawn(async move {
        turn_origin::with_origin(
            turn_origin::background_turn_origin("run-bg-1", Some("thread-bg".into())),
            g.intercept(tool, "post the report", serde_json::json!({})),
        )
        .await
    });

    let (request_id, thread_id, client_id) = tokio::time::timeout(
        Duration::from_secs(5),
        find_approval_requested(&mut event_rx, tool),
    )
    .await
    .expect("a background turn must surface an approval card, not run unasked");
    assert_eq!(thread_id.as_deref(), Some("thread-bg"));
    assert_eq!(
        client_id.as_deref(),
        Some(turn_origin::BACKGROUND_TURN_CLIENT_ID)
    );
    assert_eq!(
        gate.pending_for_thread("thread-bg").as_deref(),
        Some(request_id.as_str()),
        "a typed reply on the thread must reach the park"
    );
    let row = gate
        .list_pending()
        .unwrap()
        .into_iter()
        .find(|p| p.request_id == request_id)
        .expect("pending row persisted");
    assert_eq!(
        row.origin_class.as_deref(),
        Some("TrustedAutomation(BackgroundTurn)")
    );

    gate.decide(&request_id, ApprovalDecision::ApproveOnce)
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(outcome, GateOutcome::Allow));
}

/// With no chat thread there is nowhere to ask, so the call is denied at once —
/// no pending row, no silent park until the TTL.
#[tokio::test]
async fn background_turn_without_a_thread_is_denied_immediately() {
    let (gate, _dir) = gate();
    let outcome = tokio::time::timeout(
        Duration::from_secs(2),
        turn_origin::with_origin(
            turn_origin::background_turn_origin("run-bg-2", None),
            gate.intercept("composio_bg_turn_deny", "send", serde_json::json!({})),
        ),
    )
    .await
    .expect("must not park");
    match outcome {
        GateOutcome::Deny { reason } => {
            assert!(reason.contains(POLICY_DENIED_MARKER), "{reason}");
            assert!(reason.contains("no chat thread"), "{reason}");
        }
        other => panic!("expected an immediate deny, got {other:?}"),
    }
    assert!(gate.list_pending().unwrap().is_empty());
}

/// A sub-agent delegated from a background turn inherits the origin across
/// `tokio::spawn` (not the chat task-local) and still parks on the thread.
#[tokio::test]
async fn delegated_work_under_a_background_turn_routes_to_its_thread() {
    let (gate, _dir) = gate();
    let g = gate.clone();
    let handle = turn_origin::with_origin(
        turn_origin::background_turn_origin("run-bg-3", Some("thread-bg-3".into())),
        async move {
            turn_origin::spawn(async move {
                g.intercept("composio_bg_turn_child", "send", serde_json::json!({}))
                    .await
            })
        },
    )
    .await;

    let mut request_id = None;
    for _ in 0..1_000 {
        if let Some(id) = gate.pending_for_thread("thread-bg-3") {
            request_id = Some(id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let request_id = request_id.expect("child park routed to the background thread");
    gate.decide(&request_id, ApprovalDecision::Deny).unwrap();
    assert!(matches!(handle.await.unwrap(), GateOutcome::Deny { .. }));
}
