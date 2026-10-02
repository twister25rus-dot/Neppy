//! S7: several approvals parked on one chat thread at once.
//!
//! The gate used to keep one routing slot per thread, so a second park on the
//! same thread overwrote the first's routing. The earlier request then sat
//! unreachable — no typed reply could resolve it — until its TTL denied it, and
//! the turn waiting on it looked stuck. These tests pin that every concurrent
//! park stays routable and resolvable.

use super::*;
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

const THREAD: &str = "t-multi";

fn park(gate: &Arc<ApprovalGate>, tool: &'static str) -> tokio::task::JoinHandle<GateOutcome> {
    let g = gate.clone();
    tokio::spawn(async move {
        turn_origin::with_origin(
            AgentTurnOrigin::WebChat {
                thread_id: THREAD.into(),
                client_id: "c-multi".into(),
                request_id: None,
            },
            APPROVAL_CHAT_CONTEXT.scope(
                ApprovalChatContext {
                    thread_id: THREAD.into(),
                    client_id: "c-multi".into(),
                },
                g.intercept(tool, "do the thing", serde_json::json!({})),
            ),
        )
        .await
    })
}

async fn wait_for_parks(gate: &ApprovalGate, n: usize) -> Vec<String> {
    for _ in 0..2_000 {
        let ids = gate.pending_all_for_thread(THREAD);
        if ids.len() >= n {
            return ids;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "expected {n} parks on the thread, saw {:?}",
        gate.pending_all_for_thread(THREAD)
    );
}

/// Two concurrent parks on one thread: both are routed, both surface, and two
/// typed replies resolve both — the second reply reaches the older park instead
/// of falling through while it waits out the TTL.
#[tokio::test]
async fn two_concurrent_parks_on_one_thread_are_both_resolvable_by_reply() {
    let (gate, _dir) = gate_with_ttl(Duration::from_secs(30));

    let first = park(&gate, "composio_first");
    let ids = wait_for_parks(&gate, 1).await;
    let first_id = ids[0].clone();
    let second = park(&gate, "composio_second");
    let ids = wait_for_parks(&gate, 2).await;
    assert_eq!(ids.len(), 2, "both parks must stay routed: {ids:?}");
    assert_eq!(ids[0], first_id, "routing keeps parks oldest-first");
    let second_id = ids[1].clone();

    // Both are visible to every surface that reads the routing map.
    let routed = gate.chat_routed_request_ids();
    assert!(routed.contains(&first_id) && routed.contains(&second_id));
    let pending: Vec<String> = gate
        .list_pending()
        .unwrap()
        .into_iter()
        .map(|p| p.request_id)
        .collect();
    assert!(pending.contains(&first_id) && pending.contains(&second_id));

    // A typed reply answers the newest park (the single-slot behaviour)…
    let reply_target = gate.pending_for_thread(THREAD).expect("a routed park");
    assert_eq!(reply_target, second_id);
    gate.decide(&reply_target, ApprovalDecision::ApproveOnce)
        .unwrap()
        .expect("second park decided");
    let second_outcome = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("second park resolves promptly")
        .unwrap();
    assert!(matches!(second_outcome, GateOutcome::Allow));

    // …and the next reply reaches the older one, which is still routed.
    let reply_target = gate
        .pending_for_thread(THREAD)
        .expect("the older park must still be routable after the newer one resolves");
    assert_eq!(reply_target, first_id);
    gate.decide(&reply_target, ApprovalDecision::Deny)
        .unwrap()
        .expect("first park decided");
    let first_outcome = tokio::time::timeout(Duration::from_secs(5), first)
        .await
        .expect("first park resolves promptly, not at the TTL")
        .unwrap();
    assert!(matches!(first_outcome, GateOutcome::Deny { .. }));

    assert!(gate.pending_for_thread(THREAD).is_none());
    assert!(gate.pending_all_for_thread(THREAD).is_empty());
}

/// Deciding by request id (the approval card path) works for either park in any
/// order, and resolving the older one leaves the newer one routed.
#[tokio::test]
async fn deciding_the_older_park_by_id_keeps_the_newer_one_routed() {
    let (gate, _dir) = gate_with_ttl(Duration::from_secs(30));

    let first = park(&gate, "composio_a");
    let first_id = wait_for_parks(&gate, 1).await[0].clone();
    let second = park(&gate, "composio_b");
    let second_id = wait_for_parks(&gate, 2).await[1].clone();

    gate.decide(&first_id, ApprovalDecision::ApproveOnce)
        .unwrap()
        .expect("older park decided by id");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), first)
            .await
            .unwrap()
            .unwrap(),
        GateOutcome::Allow
    ));
    assert_eq!(
        gate.pending_all_for_thread(THREAD),
        vec![second_id.clone()],
        "only the decided park leaves the routing"
    );

    gate.decide(&second_id, ApprovalDecision::ApproveOnce)
        .unwrap()
        .expect("newer park decided by id");
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .unwrap()
            .unwrap(),
        GateOutcome::Allow
    ));
    assert!(gate.pending_all_for_thread(THREAD).is_empty());
}

/// External teardown of one park (the turn future dropped mid-park) clears only
/// its own routing entry; a sibling park on the same thread stays routable.
#[tokio::test]
async fn aborting_one_park_leaves_its_sibling_routed() {
    let (gate, _dir) = gate_with_ttl(Duration::from_secs(30));

    let first = park(&gate, "composio_x");
    let first_id = wait_for_parks(&gate, 1).await[0].clone();
    let second = park(&gate, "composio_y");
    let second_id = wait_for_parks(&gate, 2).await[1].clone();

    second.abort();
    assert!(second.await.unwrap_err().is_cancelled());
    assert_eq!(gate.pending_all_for_thread(THREAD), vec![first_id.clone()]);
    assert_eq!(
        store::get_decision(&gate.config, &second_id).unwrap(),
        Some(ApprovalDecision::Deny)
    );

    let target = gate.pending_for_thread(THREAD).expect("first still routed");
    gate.decide(&target, ApprovalDecision::ApproveOnce)
        .unwrap()
        .expect("first decided");
    assert!(matches!(first.await.unwrap(), GateOutcome::Allow));
}

/// Every concurrent park publishes its own `ApprovalRequested` (unchanged event
/// shape), so the surface can render one card per request.
#[tokio::test]
async fn every_concurrent_park_publishes_its_own_request_event() {
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS
        .get()
        .expect("event bus initialized above")
        .receiver();
    let (gate, _dir) = gate_with_ttl(Duration::from_secs(30));

    let a = park(&gate, "composio_evt_a");
    let b = park(&gate, "composio_evt_b");
    let mut seen = std::collections::HashSet::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while seen.len() < 2 {
            match rx.recv().await {
                Some(crate::core::events::DomainEvent::ApprovalRequested {
                    request_id,
                    tool_name,
                    thread_id,
                    ..
                }) if tool_name.starts_with("composio_evt_") => {
                    assert_eq!(thread_id.as_deref(), Some(THREAD));
                    seen.insert(request_id);
                }
                Some(_) => continue,
                None => panic!("bus closed"),
            }
        }
    })
    .await
    .expect("both parks must publish an ApprovalRequested");

    for id in &seen {
        gate.decide(id, ApprovalDecision::ApproveOnce).unwrap();
    }
    assert!(matches!(a.await.unwrap(), GateOutcome::Allow));
    assert!(matches!(b.await.unwrap(), GateOutcome::Allow));
}
