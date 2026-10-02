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
