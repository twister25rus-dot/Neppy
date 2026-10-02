//! Deterministic task-card dispatcher.
//!
//! Turns a [`TaskBoardCard`] into work: it **claims** the card via a
//! compare-and-set (re-load the board and transition only a `Todo`/`Ready`
//! card to `in_progress`, so a stale/concurrent re-dispatch of the same card
//! is rejected), runs a single **autonomous agent turn** toward the card's
//! objective, and **writes the outcome back** to the board (`done` + evidence
//! on success, `blocked` + reason on failure).
//!
//! This is the one executor both dispatch paths converge on:
//! - the **board poller** (cards that arrived without a proactive trigger), and
//! - the **proactive triage** arm (`agent::triage::apply_decision`), once it has
//!   decided to act on a task-board card.
//!
//! The runner mirrors `skills::spawn_workflow_run_background`: build the
//! `orchestrator` agent fresh inside a detached task, cap tool iterations, and
//! run `agent.run_single` under `with_autonomous_iter_cap`. PR-4 generalises the
//! executor from the default agent to a resolved personality/skill; this module
//! keeps the default-agent path so the pipeline runs end-to-end first.

mod dispatch;
mod executor;
mod poller;
mod prompt;
mod registry;
mod types;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod origin_tests;

// ── Public API ────────────────────────────────────────────────────────────────

pub use dispatch::dispatch_card;
pub use poller::start_board_poller;
pub use prompt::build_task_prompt;
pub use registry::{cancel_session, cancel_session_scoped};
pub use types::DispatchOutcome;

/// Run a one-off **system** agent turn on an existing chat thread, streaming the
/// result into it like a normal assistant turn (the same web-channel bridge
/// cron / welcome agents use). Used by the background-completion delivery
/// subsystem to surface a finished detached sub-agent's result back into the
/// chat. Best-effort: returns the final response text or an error string.
pub async fn run_system_turn_on_thread(
    thread_id: String,
    prompt: String,
) -> Result<String, String> {
    let config = crate::neppy::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e:#}"))?;
    let executor = executor::resolve_executor(&config.workspace_dir, None);
    let run_id = format!("bgdeliver-{}", uuid::Uuid::new_v4());
    executor::run_autonomous(config, &executor, &prompt, &run_id, Some(thread_id)).await
}

/// A detached Pet companion hand-off run, as returned by
/// [`run_pet_companion_handoff`].
pub struct PetCompanionHandoff {
    /// System-generated run id (`pet-handoff-<uuid>`), also the event context.
    pub run_id: String,
    /// The run's own top-level `tasks` thread, or `None` when the thread store
    /// refused the create (the run then proceeds headless and approvals surface
    /// in the Pet inbox only).
    pub thread_id: Option<String>,
    /// Resolves once with the run's final response, or its error string.
    pub result: tokio::sync::oneshot::Receiver<Result<String, String>>,
    /// Aborts the run (e.g. the user cancels the hand-off from the Pet). An
    /// aborted run drops `result`'s sender, so the receiver sees `RecvError`.
    pub abort: tokio::task::AbortHandle,
}

/// Run a task the Pet desktop companion handed off, in its **own named
/// thread**, under the
/// [`PetCompanion`](crate::neppy::agent::turn_origin::TrustedAutomationSource::PetCompanion)
/// approval origin — never `Cli`, never a trust root. Spawns and returns at
/// once; await [`PetCompanionHandoff::result`] for the outcome.
///
/// * The thread is a top-level `tasks` thread titled `title`, seeded with
///   `prompt`; the run streams into it like a task-board card run and appends
///   its final response.
/// * The executor is the default agent (`orchestrator`), with the autonomous
///   task-run iteration budget.
/// * Every `external_effect` call follows the user's normal approval settings;
///   a high-risk action class always parks for confirmation. Parks surface in
///   the Pet inbox and as a card on the hand-off thread (see
///   `security::approval::gate::pet_companion_high_risk`).
///
/// `job_id` is the companion's system-generated id for the work (e.g.
/// `pet-companion:<suggestion_id>`), carried on the origin for audit logs; it
/// must never contain observed text. The prompt should already be scrubbed.
/// Errors (nothing spawned) on a blank prompt.
pub async fn run_pet_companion_handoff(
    config: crate::neppy::config::Config,
    job_id: &str,
    title: &str,
    prompt: &str,
) -> Result<PetCompanionHandoff, String> {
    if prompt.trim().is_empty() {
        return Err("hand-off prompt is empty".to_string());
    }
    let executor = executor::resolve_executor(&config.workspace_dir, None);
    let run_id = format!("pet-handoff-{}", uuid::Uuid::new_v4());
    let thread_id = crate::neppy::agent::task_session::create_named_session_thread(
        config.workspace_dir.clone(),
        title,
        &run_id,
        prompt,
    );
    let origin = crate::neppy::agent::turn_origin::pet_companion_origin(job_id, thread_id.clone());
    tracing::info!(
        run_id = %run_id,
        has_thread = thread_id.is_some(),
        agent_id = %executor.agent_id,
        origin_class = %origin.class(),
        "[task_dispatcher] pet companion hand-off: spawning run"
    );

    let (tx, rx) = tokio::sync::oneshot::channel();
    let task_run_id = run_id.clone();
    let task_thread = thread_id.clone();
    let prompt = prompt.to_string();
    // The run scopes its own `PetCompanion` origin inside; it must not inherit
    // whatever origin (if any) the companion runtime's task carries.
    let join = crate::neppy::agent::turn_origin::spawn_unlabelled(
        "pet companion hand-off runs under its own PetCompanion origin",
        async move {
            let outcome = executor::run_autonomous_with_origin(
                config,
                &executor,
                &prompt,
                &task_run_id,
                task_thread,
                origin,
            )
            .await;
            tracing::info!(
                run_id = %task_run_id,
                ok = outcome.is_ok(),
                "[task_dispatcher] pet companion hand-off finished"
            );
            let _ = tx.send(outcome);
        },
    );
    Ok(PetCompanionHandoff {
        run_id,
        thread_id,
        result: rx,
        abort: join.abort_handle(),
    })
}
