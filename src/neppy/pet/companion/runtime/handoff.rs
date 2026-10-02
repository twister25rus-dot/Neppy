//! Hand-off: a task the user (or a level-3 trusted category) gives to a
//! background orchestration run, in its own thread, under the `PetCompanion`
//! approval origin (`task_dispatcher::run_pet_companion_handoff`). Every
//! external effect in that run follows the user's approval settings and
//! high-risk classes always park. The companion keeps the abort handle, and
//! the result comes back into the suggestion scrubbed and capped.

use std::sync::Arc;

use async_trait::async_trait;

use super::bus::{self, CompanionUiEvent};
use super::state::Runtime;
use crate::neppy::config::Config;
use crate::neppy::pet::companion::store::{self, MAX_RESULT_EXCERPT};
use crate::neppy::pet::companion::types::{CompanionSuggestion, Handoff, HandoffStatus};

/// A started run.
pub struct StartedHandoff {
    pub run_id: String,
    pub thread_id: Option<String>,
    pub result: tokio::sync::oneshot::Receiver<Result<String, String>>,
    pub abort: tokio::task::AbortHandle,
}

/// The executor seam (production: the task dispatcher; tests: a fake).
#[async_trait]
pub trait HandoffRunner: Send + Sync {
    async fn start(
        &self,
        config: Config,
        job_id: &str,
        title: &str,
        prompt: &str,
    ) -> Result<StartedHandoff, String>;
}

pub struct DispatcherHandoff;

#[async_trait]
impl HandoffRunner for DispatcherHandoff {
    async fn start(
        &self,
        config: Config,
        job_id: &str,
        title: &str,
        prompt: &str,
    ) -> Result<StartedHandoff, String> {
        let h = crate::neppy::agent::task_dispatcher::run_pet_companion_handoff(
            config, job_id, title, prompt,
        )
        .await?;
        Ok(StartedHandoff {
            run_id: h.run_id,
            thread_id: h.thread_id,
            result: h.result,
            abort: h.abort,
        })
    }
}

/// Start a hand-off for `sugg` with the (already scrubbed) `prompt`. Returns
/// the suggestion with `handoff.status = running`.
pub async fn start(
    rt: &Arc<Runtime>,
    config: &Config,
    sugg: &CompanionSuggestion,
    prompt: &str,
) -> Result<CompanionSuggestion, String> {
    let title: String = format!("Pet: {}", sugg.headline)
        .chars()
        .take(120)
        .collect();
    let job_id = format!("pet-companion:{}", sugg.id);
    let started = rt
        .handoff
        .start(config.clone(), &job_id, &title, prompt)
        .await?;
    let thread_id = started
        .thread_id
        .clone()
        .unwrap_or_else(|| started.run_id.clone());
    log::info!(
        "[pet::companion] hand-off started id={} has_thread={}",
        sugg.id,
        started.thread_id.is_some()
    );
    let running = Handoff {
        thread_id: thread_id.clone(),
        status: HandoffStatus::Running,
        result_excerpt: None,
    };
    store::set_handoff(config, &sugg.id, &running).map_err(|e| format!("{e:#}"))?;
    rt.lock()
        .handoffs
        .insert(sugg.id.clone(), started.abort.clone());

    let rt2 = rt.clone();
    let config2 = config.clone();
    let id = sugg.id.clone();
    let has_thread = started.thread_id.is_some();
    let result = started.result;
    let finish = async move {
        let outcome = result.await;
        rt2.lock().handoffs.remove(&id);
        let (status, excerpt) = match outcome {
            Ok(Ok(text)) => (HandoffStatus::Done, Some(text)),
            // The error text may quote the run's content: never logged.
            Ok(Err(_)) => (HandoffStatus::Failed, None),
            Err(_) => (HandoffStatus::Failed, None),
        };
        // `set_handoff` scrubs and caps the excerpt before it is stored.
        let excerpt = excerpt.map(|t| t.chars().take(MAX_RESULT_EXCERPT * 4).collect::<String>());
        let done = Handoff {
            thread_id,
            status,
            result_excerpt: excerpt,
        };
        if let Err(e) = store::set_handoff(&config2, &id, &done) {
            log::warn!("[pet::companion] hand-off result write failed: {e:#}");
            return;
        }
        log::info!(
            "[pet::companion] hand-off finished id={id} status={}",
            status.as_str()
        );
        if let Ok(Some(s)) = store::get_suggestion(&config2, &id) {
            bus::publish(CompanionUiEvent::SuggestionUpdate { suggestion: s });
        }
        if has_thread && status == HandoffStatus::Done {
            notify_done(&done.thread_id);
        }
    };
    match rt.tokio() {
        Some(h) => {
            h.spawn(finish);
        }
        None => {
            tokio::spawn(finish);
        }
    }
    store::get_suggestion(config, &sugg.id)
        .map_err(|e| format!("{e:#}"))?
        .ok_or_else(|| "suggestion vanished".to_string())
}

/// Content-free notification with a deep link to the hand-off thread.
fn notify_done(thread_id: &str) {
    use crate::neppy::desktop::notifications::{
        publish_core_notification, CoreNotificationCategory, CoreNotificationEvent,
    };
    let now = chrono::Utc::now();
    publish_core_notification(CoreNotificationEvent {
        id: format!("pet-companion-handoff:{}", now.timestamp_millis()),
        category: CoreNotificationCategory::Agents,
        title: "Pet hand-off finished".into(),
        body: "Open the thread to see the result.".into(),
        deep_link: Some(format!("/chat/{thread_id}")),
        timestamp_ms: now.timestamp_millis().max(0) as u64,
        actions: None,
    });
}
