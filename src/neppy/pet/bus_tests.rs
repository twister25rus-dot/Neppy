use chrono::Utc;
use tempfile::TempDir;

use super::bus::*;
use super::store;
use super::store_feed;
use super::store_notes;
use super::store_tests::{new_note, test_config};
use super::types::*;
use crate::core::events::DomainEvent;
use crate::neppy::security::approval::{ApprovalSourceContext, PendingApproval};

fn completed(job_id: &str, agent_id: Option<&str>) -> DomainEvent {
    DomainEvent::CronJobCompleted {
        job_id: job_id.into(),
        success: true,
        output: "Recorded 1 notes.".into(),
        agent_id: agent_id.map(str::to_string),
    }
}

#[tokio::test]
async fn pet_pass_completion_runs_surfacing_and_others_are_ignored() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let pet = store::ensure_primary(&config, Utc::now()).unwrap();
    store::set_research_job_id(&config, &pet.id, Some("job-1")).unwrap();
    store_notes::insert_note(&config, &new_note(&pet.id, "x"), false, Utc::now()).unwrap();

    // Another agent, or no agent, never surfaces.
    assert!(!handle_pass_completed(&config, &completed("job-1", Some("morning_briefing"))).await);
    assert!(!handle_pass_completed(&config, &completed("job-1", None)).await);
    // A pet_research completion for a job no pet owns is ignored too.
    assert!(
        !handle_pass_completed(&config, &completed("other", Some(PET_RESEARCH_AGENT_ID))).await
    );
    assert_eq!(store_notes::new_notes(&config, &pet.id).unwrap().len(), 1);

    assert!(handle_pass_completed(&config, &completed("job-1", Some(PET_RESEARCH_AGENT_ID))).await);
    assert!(store_notes::new_notes(&config, &pet.id).unwrap().is_empty());
    let run = store_feed::last_run(&config, &pet.id).unwrap().unwrap();
    assert_eq!(run.trigger, "scheduled");
    assert_eq!(run.notes_seen, 1);
}

fn requested(request_id: &str, thread_id: Option<&str>) -> DomainEvent {
    DomainEvent::ApprovalRequested {
        request_id: request_id.into(),
        tool_name: "composio_execute".into(),
        action_summary: "send 1 email".into(),
        args_redacted: serde_json::json!({}),
        thread_id: thread_id.map(str::to_string),
        client_id: None,
    }
}

#[test]
fn background_approval_without_thread_or_flow_is_surfaced() {
    let plain = PendingApproval::new(
        "r1",
        "composio_execute",
        "send 1 email",
        serde_json::json!({}),
        None,
    )
    .with_origin_class("TrustedAutomation(GoalContinuation)");
    let flow = PendingApproval::new(
        "r2",
        "composio_execute",
        "send",
        serde_json::json!({}),
        None,
    )
    .with_source_context(ApprovalSourceContext::Flow {
        flow_id: "f".into(),
        run_id: "run".into(),
        node_id: None,
    });
    let pending = vec![plain, flow];

    match approval_needs_surface(&requested("r1", None), &pending) {
        Some(DomainEvent::PetApprovalNeeded {
            request_id,
            tool_name,
            action_summary,
        }) => {
            assert_eq!(request_id, "r1");
            assert_eq!(tool_name, "composio_execute");
            assert_eq!(action_summary, "send 1 email");
        }
        other => panic!("expected PetApprovalNeeded, got {other:?}"),
    }
    // Chat-routed requests already have a card.
    assert!(approval_needs_surface(&requested("r1", Some("thread")), &pending).is_none());
    // Flow parks publish their own notification.
    assert!(approval_needs_surface(&requested("r2", None), &pending).is_none());
    // Unknown (already decided) requests are ignored.
    assert!(approval_needs_surface(&requested("gone", None), &pending).is_none());
    // Other events are ignored.
    assert!(approval_needs_surface(&completed("j", None), &pending).is_none());
}

#[test]
fn remote_and_unknown_origin_approvals_are_not_surfaced() {
    let row = |id: &str| PendingApproval::new(id, "shell", "run ls", serde_json::json!({}), None);
    let pending = vec![
        row("remote").with_origin_class("ExternalChannel(telegram)"),
        row("local").with_origin_class("TrustedAutomation(GoalContinuation)"),
        row("legacy"), // no recorded origin: unknown, fails closed
    ];
    assert!(
        approval_needs_surface(&requested("remote", None), &pending).is_none(),
        "an ExternalChannel park keeps its silent TTL-deny"
    );
    assert!(approval_needs_surface(&requested("legacy", None), &pending).is_none());
    assert!(approval_needs_surface(&requested("local", None), &pending).is_some());
}

#[test]
fn subscriber_names_and_domains() {
    use tinybus::EventHandler;
    assert_eq!(PetPassCompletedSubscriber.name(), "pet::pass_completed");
    assert_eq!(PetPassCompletedSubscriber.domains(), Some(&["cron"][..]));
    assert_eq!(PetApprovalSurfaceSubscriber.name(), "pet::approval_surface");
    assert_eq!(
        PetApprovalSurfaceSubscriber.domains(),
        Some(&["approval"][..])
    );
}
