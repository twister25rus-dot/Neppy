//! M1: the origin an autonomous run executes under.
//!
//! Task-board card runs and background-delivery follow-ups used to run as
//! `AgentTurnOrigin::Cli`, which the approval gate allows without a prompt.
//! They now run as a `BackgroundTurn` on their chat thread (the gate-side
//! behaviour is pinned in `security::approval::gate_background_turn_tests`).

use crate::neppy::agent::turn_origin::{AgentTurnOrigin, TrustedAutomationSource};

use super::executor::background_run_origin;

#[test]
fn autonomous_run_with_a_session_thread_is_a_background_turn_on_it() {
    match background_run_origin("run-42", Some("thread-42")) {
        AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::BackgroundTurn { thread_id },
        } => {
            assert_eq!(job_id, "run-42");
            assert_eq!(thread_id.as_deref(), Some("thread-42"));
        }
        other => panic!("expected a BackgroundTurn origin, got {other:?}"),
    }
}

#[test]
fn headless_autonomous_run_is_a_background_turn_with_no_thread() {
    let origin = background_run_origin("run-43", None);
    assert!(
        matches!(
            origin,
            AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::BackgroundTurn { thread_id: None },
                ..
            }
        ),
        "a headless run must not be a trust root, got {origin:?}"
    );
    assert!(
        !matches!(origin, AgentTurnOrigin::Cli),
        "autonomous runs must never be labelled Cli (gate allows Cli unasked)"
    );
}

// ── Pet companion hand-off (T2b) ─────────────────────────────────────────────

use super::executor::scope_autonomous_run;
use crate::neppy::agent::tinyagents::thread_context;
use crate::neppy::agent::turn_origin;

/// The hand-off executor's scoping seam: the run (and its thread context) sees
/// exactly the `PetCompanion` origin it was given — not `Cli`, not a
/// `BackgroundTurn` — with the hand-off thread on it.
#[tokio::test]
async fn handoff_run_executes_under_the_pet_companion_origin() {
    let origin = turn_origin::pet_companion_origin("pet-companion:s-1", Some("task-h1".into()));
    let (seen, thread) = scope_autonomous_run(origin, None, Some("task-h1".into()), async {
        (turn_origin::current(), thread_context::current_thread_id())
    })
    .await;
    match seen {
        Some(AgentTurnOrigin::TrustedAutomation {
            job_id,
            source: TrustedAutomationSource::PetCompanion { thread_id },
        }) => {
            assert_eq!(job_id, "pet-companion:s-1");
            assert_eq!(thread_id.as_deref(), Some("task-h1"));
        }
        other => panic!("expected a PetCompanion origin, got {other:?}"),
    }
    assert_eq!(thread.as_deref(), Some("task-h1"));
    // Nothing leaks out of the scope.
    assert!(turn_origin::current().is_none());
}

/// A sub-agent the hand-off delegates to inherits the same origin across a
/// spawn (so its external-effect calls are gated the same way).
#[tokio::test]
async fn delegated_work_inside_a_handoff_keeps_the_companion_origin() {
    let origin = turn_origin::pet_companion_origin("pet-companion:s-2", None);
    let seen = scope_autonomous_run(origin, None, None, async {
        turn_origin::spawn(async { turn_origin::current() })
            .await
            .expect("spawned task panicked")
    })
    .await;
    assert!(
        matches!(
            seen,
            Some(AgentTurnOrigin::TrustedAutomation {
                source: TrustedAutomationSource::PetCompanion { thread_id: None },
                ..
            })
        ),
        "got {seen:?}"
    );
}

/// The card-run path still scopes a `BackgroundTurn` through the same seam.
#[tokio::test]
async fn card_runs_still_scope_a_background_turn() {
    let seen = scope_autonomous_run(
        background_run_origin("run-44", Some("thread-44")),
        None,
        Some("thread-44".into()),
        async { turn_origin::current() },
    )
    .await;
    assert!(matches!(
        seen,
        Some(AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::BackgroundTurn { .. },
            ..
        })
    ));
}

#[tokio::test]
async fn a_blank_handoff_prompt_spawns_nothing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = crate::neppy::config::Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..crate::neppy::config::Config::default()
    };
    let err = super::run_pet_companion_handoff(config, "pet-companion:s-3", "Pet: x", "   ")
        .await
        .err()
        .expect("blank prompt refused");
    assert!(err.contains("empty"), "{err}");
    assert!(
        tinycortex::memory::conversations::list_threads(tmp.path().to_path_buf())
            .map(|t| t.is_empty())
            .unwrap_or(true),
        "no thread created"
    );
}

/// Release re-review (round 3, item 4): the hand-off thread is created WITH the
/// Pet origin marker in one write (no window in which a follow-up turn could
/// see an unmarked hand-off thread), and the marker is hidden from clients.
#[test]
fn handoff_thread_is_created_with_the_pet_marker() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = super::create_pet_handoff_thread(tmp.path(), "Pet: x", "run-1", "do it").unwrap();
    let threads =
        tinycortex::memory::conversations::list_threads(tmp.path().to_path_buf()).unwrap();
    let t = threads.into_iter().find(|t| t.id == id).expect("thread");
    assert!(crate::neppy::threads::mode::is_pet_companion_thread(
        &t.labels
    ));
    assert!(t.labels.iter().any(|l| l == "tasks"));
    assert_eq!(
        crate::neppy::threads::mode::strip_reserved_labels(t.labels),
        vec!["tasks".to_string()]
    );
}

/// Item 4, fail closed: when the marked thread cannot be created, the hand-off
/// is aborted — nothing runs without the marker.
#[tokio::test]
async fn handoff_aborts_when_the_marked_thread_cannot_be_created() {
    let tmp = tempfile::TempDir::new().unwrap();
    // A regular file where the workspace directory should be: every store
    // write under it fails.
    let blocker = tmp.path().join("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();
    let config = crate::neppy::config::Config {
        workspace_dir: blocker.clone(),
        ..crate::neppy::config::Config::default()
    };
    let err = super::run_pet_companion_handoff(config, "pet-companion:s-4", "Pet: x", "go")
        .await
        .err()
        .expect("hand-off must abort without a marked thread");
    assert!(err.contains("hand-off thread"), "{err}");
}
