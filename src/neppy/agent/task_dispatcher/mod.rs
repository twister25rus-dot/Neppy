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
///
/// The turn honours the thread: it runs in the thread's saved mode (so an
/// Orchestration thread keeps its supervisor surface), and on a Pet companion
/// hand-off thread it runs under the Pet companion approval origin instead of
/// a `BackgroundTurn` (see [`follow_up_turn_context`]).
pub async fn run_system_turn_on_thread(
    thread_id: String,
    prompt: String,
) -> Result<String, String> {
    let config = crate::neppy::config::Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e:#}"))?;
    let executor = executor::resolve_executor(&config.workspace_dir, None);
    let run_id = format!("bgdeliver-{}", uuid::Uuid::new_v4());
    let fallback = executor::background_run_origin(&run_id, Some(&thread_id));
    let (origin, mode) =
        follow_up_turn_context(&config.workspace_dir, &thread_id, &run_id, fallback).await;
    crate::neppy::threads::mode::with_turn_mode(
        mode,
        Box::pin(executor::run_autonomous_with_origin(
            config,
            &executor,
            &prompt,
            &run_id,
            Some(thread_id),
            origin,
        )),
    )
    .await
}

/// The approval origin and thread mode for an unattended follow-up turn on an
/// existing thread (background delivery, goal continuation).
///
/// * On a Pet companion hand-off thread (marked
///   [`PET_COMPANION_THREAD_LABEL`](crate::neppy::threads::mode::PET_COMPANION_THREAD_LABEL)
///   when the hand-off created it) the turn runs under the Pet companion
///   origin with that thread, never `fallback`: a follow-up turn is still Pet
///   work, so its high-risk actions must always be confirmed.
/// * Otherwise it runs under `fallback`.
/// * The mode is the thread's saved mode (chat when unset or unreadable).
pub(crate) async fn follow_up_turn_context(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    job_id: &str,
    fallback: crate::neppy::agent::turn_origin::AgentTurnOrigin,
) -> (
    crate::neppy::agent::turn_origin::AgentTurnOrigin,
    crate::neppy::threads::mode::ThreadMode,
) {
    use crate::neppy::threads::mode::{is_pet_companion_thread, ThreadMode};
    let labels = crate::neppy::threads::ops::thread_labels_for(workspace_dir, thread_id)
        .await
        .unwrap_or_default();
    let mode = ThreadMode::from_labels(&labels);
    if is_pet_companion_thread(&labels) {
        let origin = crate::neppy::agent::turn_origin::pet_companion_origin(
            &format!(
                "{}{job_id}",
                crate::neppy::security::approval::gate::PET_FOLLOW_UP_JOB_PREFIX
            ),
            Some(thread_id.to_string()),
        );
        tracing::info!(
            job_id = %job_id,
            mode = %mode,
            origin_class = %origin.class(),
            "[task_dispatcher] follow-up turn on a Pet hand-off thread: Pet companion origin"
        );
        return (origin, mode);
    }
    tracing::debug!(
        job_id = %job_id,
        mode = %mode,
        origin_class = %fallback.class(),
        "[task_dispatcher] follow-up turn origin"
    );
    (fallback, mode)
}

/// Creates the Pet hand-off's top-level `tasks` thread **with** the reserved
/// [`PET_COMPANION_THREAD_LABEL`](crate::neppy::threads::mode::PET_COMPANION_THREAD_LABEL)
/// in the same write (hidden from clients, kept across label edits), seeded
/// with `prompt`. `Err` when the thread cannot be created — the caller then
/// aborts the hand-off rather than run without the marker.
fn create_pet_handoff_thread(
    workspace_dir: &std::path::Path,
    title: &str,
    run_id: &str,
    prompt: &str,
) -> Result<String, String> {
    crate::neppy::agent::task_session::create_named_session_thread(
        workspace_dir.to_path_buf(),
        title,
        run_id,
        prompt,
        &[crate::neppy::threads::mode::PET_COMPANION_THREAD_LABEL],
    )
    .ok_or_else(|| {
        tracing::warn!("[task_dispatcher] pet hand-off thread create failed — aborting hand-off");
        "could not create the hand-off thread; the hand-off was not started".to_string()
    })
}

/// Marks an existing thread as a Pet companion hand-off thread (test helper;
/// production creates the thread with the marker, see
/// [`create_pet_handoff_thread`]).
#[cfg(test)]
pub(crate) fn mark_pet_companion_thread(workspace_dir: &std::path::Path, thread_id: &str) -> bool {
    use crate::neppy::threads::mode::PET_COMPANION_THREAD_LABEL;
    use tinycortex::memory::conversations as store;
    let mut labels = match store::list_threads(workspace_dir.to_path_buf()) {
        Ok(threads) => match threads.into_iter().find(|t| t.id == thread_id) {
            Some(t) => t.labels,
            None => {
                tracing::warn!("[task_dispatcher] hand-off thread missing; not marked");
                return false;
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "[task_dispatcher] could not read threads; hand-off thread not marked");
            return false;
        }
    };
    if labels.iter().any(|l| l == PET_COMPANION_THREAD_LABEL) {
        return true;
    }
    labels.push(PET_COMPANION_THREAD_LABEL.to_string());
    match store::update_thread_labels(
        workspace_dir.to_path_buf(),
        thread_id,
        labels,
        &chrono::Utc::now().to_rfc3339(),
    ) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, "[task_dispatcher] could not mark the hand-off thread");
            false
        }
    }
}

/// A detached Pet companion hand-off run, as returned by
/// [`run_pet_companion_handoff`].
pub struct PetCompanionHandoff {
    /// System-generated run id (`pet-handoff-<uuid>`), also the event context.
    pub run_id: String,
    /// The run's own top-level `tasks` thread, created with the Pet hand-off
    /// marker. Always `Some` now: a hand-off whose thread cannot be created is
    /// aborted (fail closed). Kept an `Option` for the companion's runner seam.
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
    // Fail closed: the thread is created WITH the Pet hand-off marker in one
    // write, and no thread means no run. A hand-off whose follow-up turns could
    // lose the Pet companion origin must not start.
    let thread_id = Some(create_pet_handoff_thread(
        &config.workspace_dir,
        title,
        &run_id,
        prompt,
    )?);
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
