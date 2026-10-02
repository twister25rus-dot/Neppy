//! Terminal-event backstop for a detached async sub-agent (M3).
//!
//! `spawn_async_subagent` runs the child on its own task and reports every
//! terminal outcome from *inside* that task: a completion, a failure, an
//! awaiting-input pause. A panic skips all of it — the task unwinds past the
//! reporting code — and the `JoinHandle` that would have carried the panic was
//! dropped, so nothing reached the parent chat (no `background_completions`
//! entry for delivery) or the UI (no `SubagentFailed` event). The lifecycle
//! store saw the dropped status sender and recorded a failure, but no one was
//! told.
//!
//! [`watch_detached_run`] keeps the handle and, on a panic, emits exactly the
//! failure the in-task error path would have: the durable session is marked
//! failed, a `[SUBAGENT_FAILED]` notice is queued for idle delivery into the
//! parent thread, `SubagentFailed` is published, and the parent's progress sink
//! (when attached) gets `AgentProgress::SubagentFailed`.
//!
//! A *cancelled* join is not reported: cancellation is deliberate (the user
//! closed the sub-agent or deleted its thread) and the cancelling path owns any
//! messaging, so reporting it here would announce a failure nobody had.

use tokio::task::{JoinError, JoinHandle};

use crate::neppy::agent::orchestration::subagent_sessions::{self, SubagentSessionStore};
use crate::neppy::agent::progress::AgentProgress;

/// Who to tell when a detached sub-agent's task dies abnormally.
pub(super) struct DetachedRunReport {
    pub(super) parent_session: String,
    pub(super) task_id: String,
    pub(super) agent_id: String,
    pub(super) parent_thread_id: Option<String>,
    /// Durable session to mark failed, when the spawn created one.
    pub(super) session: Option<(SubagentSessionStore, String)>,
    pub(super) progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
}

/// Watch `join` and emit the terminal failure a panic would otherwise swallow.
/// Returns the watcher's own handle (tests await it; production drops it — the
/// watcher is self-contained and ends when the child does).
///
/// Take the child's `abort_handle()` **before** calling this: the watcher owns
/// the `JoinHandle`, and aborting through the abort handle still reaches the
/// child (the watcher then sees a cancelled join and stays quiet).
pub(super) fn watch_detached_run(
    join: JoinHandle<()>,
    report: DetachedRunReport,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        match join.await {
            Ok(()) => {
                log::debug!(
                    "[spawn_async_subagent] detached task_id={} ended normally (in-task reporting owned the outcome)",
                    report.task_id
                );
            }
            Err(err) if err.is_cancelled() => {
                log::debug!(
                    "[spawn_async_subagent] detached task_id={} was cancelled; the cancelling path owns reporting",
                    report.task_id
                );
            }
            Err(err) => report_panic(report, err).await,
        }
    })
}

async fn report_panic(report: DetachedRunReport, err: JoinError) {
    let detail = panic_detail(err);
    let error = format!("the sub-agent crashed unexpectedly ({detail})");
    log::error!(
        "[spawn_async_subagent] detached task_id={} agent_id={} panicked — reporting failure to parent + UI",
        report.task_id,
        report.agent_id
    );
    if let Some((store, subagent_session_id)) = &report.session {
        if let Err(store_err) = subagent_sessions::mark_failed(
            store,
            subagent_session_id,
            &report.task_id,
            error.clone(),
        ) {
            log::warn!(
                "[subagent_reuse] mark_failed after panic failed subagent_session_id={} task_id={} error={}",
                subagent_session_id,
                report.task_id,
                store_err
            );
        }
    }
    crate::neppy::agent::orchestration::background_completions::record_failure(
        report.parent_session.clone(),
        report.task_id.clone(),
        report.agent_id.clone(),
        &error,
        report.parent_thread_id.clone(),
    );
    crate::neppy::agent::orchestration::subagent_events::publish_subagent_failed(
        report.parent_session.clone(),
        report.task_id.clone(),
        report.agent_id.clone(),
        error.clone(),
    );
    if let Some(tx) = &report.progress {
        let _ = tx
            .send(AgentProgress::SubagentFailed {
                agent_id: report.agent_id.clone(),
                task_id: report.task_id.clone(),
                error,
            })
            .await;
    }
}

/// The panic payload as text, when it is one (`panic!("…")` payloads are
/// `&str` or `String`); otherwise a generic label. Bounded so a huge payload
/// cannot flood the chat notice.
fn panic_detail(err: JoinError) -> String {
    let payload = match err.try_into_panic() {
        Ok(payload) => payload,
        Err(_) => return "task failed".to_string(),
    };
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-text panic payload".to_string());
    crate::neppy::util::truncate_with_ellipsis(&text, 300)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neppy::agent::orchestration::background_completions::{
        take_pending, BackgroundAgentOutcome,
    };

    fn report(
        session: &str,
        task: &str,
    ) -> (
        DetachedRunReport,
        tokio::sync::mpsc::Receiver<AgentProgress>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        (
            DetachedRunReport {
                parent_session: session.to_string(),
                task_id: task.to_string(),
                agent_id: "researcher".to_string(),
                parent_thread_id: Some("thread-panic".to_string()),
                session: None,
                progress: Some(tx),
            },
            rx,
        )
    }

    /// M3: a panicking detached child reaches the parent (a queued
    /// `[SUBAGENT_FAILED]` delivery) and the UI (a `SubagentFailed` progress
    /// event). Before the watcher, the dropped `JoinHandle` swallowed the panic.
    #[tokio::test]
    async fn panicking_detached_subagent_is_reported_to_parent_and_ui() {
        let session = "sess-m3-panic";
        let (report, mut progress) = report(session, "sub-m3-panic");
        let child = tokio::spawn(async {
            panic!("fake sub-agent exploded");
        });

        watch_detached_run(child, report).await.unwrap();

        let pending = take_pending(session);
        assert_eq!(pending.len(), 1, "a failure must be queued for delivery");
        assert_eq!(pending[0].task_id, "sub-m3-panic");
        assert_eq!(pending[0].outcome, BackgroundAgentOutcome::Failed);
        assert!(pending[0].summary.starts_with("[SUBAGENT_FAILED]"));
        assert!(
            pending[0].summary.contains("fake sub-agent exploded"),
            "{}",
            pending[0].summary
        );
        assert_eq!(pending[0].parent_thread_id.as_deref(), Some("thread-panic"));

        match progress.try_recv() {
            Ok(AgentProgress::SubagentFailed {
                task_id, agent_id, ..
            }) => {
                assert_eq!(task_id, "sub-m3-panic");
                assert_eq!(agent_id, "researcher");
            }
            other => panic!("expected SubagentFailed progress, got {other:?}"),
        }
    }

    /// A normal finish and a deliberate cancel are reported by their own paths;
    /// the watcher must not add a phantom failure.
    #[tokio::test]
    async fn normal_finish_and_cancel_are_not_reported() {
        let session = "sess-m3-quiet";
        let (ok_report, mut ok_progress) = report(session, "sub-m3-ok");
        watch_detached_run(tokio::spawn(async {}), ok_report)
            .await
            .unwrap();

        let (cancel_report, mut cancel_progress) = report(session, "sub-m3-cancel");
        let child = tokio::spawn(std::future::pending::<()>());
        let abort = child.abort_handle();
        let watcher = watch_detached_run(child, cancel_report);
        abort.abort();
        watcher.await.unwrap();

        assert!(take_pending(session).is_empty());
        assert!(ok_progress.try_recv().is_err());
        assert!(cancel_progress.try_recv().is_err());
    }
}
