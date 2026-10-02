//! Event-bus subscribers for Pet mode.
//!
//! * [`PetPassCompletedSubscriber`] — runs surfacing after a scheduled
//!   research pass (`CronJobCompleted` with `agent_id == pet_research`).
//!   Manual passes (`pet_run_now`) surface inline and never publish
//!   `CronJobCompleted`, so a pass is never surfaced twice.
//! * [`PetApprovalSurfaceSubscriber`] — a background approval parked with no
//!   chat thread and no flow context would otherwise silently TTL-deny; this
//!   republishes it as `PetApprovalNeeded` so the notification bridge persists
//!   a notification linking to the Pet inbox.

use async_trait::async_trait;
use tinybus::EventHandler;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::neppy::config::Config;
use crate::neppy::security::approval::PendingApproval;

use super::store;
use super::surface;
use super::types::PET_RESEARCH_AGENT_ID;

/// Surfaces a scheduled Pet pass.
pub struct PetPassCompletedSubscriber;

/// Handle one event against `config`. Returns whether surfacing ran.
pub(crate) async fn handle_pass_completed(config: &Config, event: &DomainEvent) -> bool {
    let DomainEvent::CronJobCompleted {
        job_id,
        success,
        agent_id: Some(agent_id),
        ..
    } = event
    else {
        return false;
    };
    if agent_id != PET_RESEARCH_AGENT_ID {
        return false;
    }
    let pet_id = match store::find_pet_by_job(config, job_id) {
        Ok(Some(id)) => id,
        Ok(None) => {
            log::debug!("[pet::bus] pass completed for an unowned job job_id={job_id}");
            return false;
        }
        Err(e) => {
            log::warn!("[pet::bus] pet lookup failed: {e}");
            return false;
        }
    };
    log::debug!("[pet::bus] scheduled pass completed pet_id={pet_id} success={success}");
    match surface::surface_after_pass(config, &pet_id, Some(job_id), "scheduled", *success).await {
        Ok(_) => true,
        Err(e) => {
            log::warn!("[pet::bus] surfacing failed pet_id={pet_id}: {e}");
            false
        }
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for PetPassCompletedSubscriber {
    fn name(&self) -> &str {
        "pet::pass_completed"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // Cheap pre-filter so the config load only happens for pet passes.
        if !matches!(
            event,
            DomainEvent::CronJobCompleted { agent_id: Some(a), .. } if a == PET_RESEARCH_AGENT_ID
        ) {
            return;
        }
        match crate::neppy::config::rpc::load_config_with_timeout().await {
            Ok(config) => {
                handle_pass_completed(&config, event).await;
            }
            Err(e) => log::warn!("[pet::bus] config load failed: {e}"),
        }
    }
}

/// Surfaces background approvals that have no chat card.
pub struct PetApprovalSurfaceSubscriber;

/// The `PetApprovalNeeded` event for `event`, when it is a background park:
/// no chat thread, a pending row with no flow context (flows publish their own
/// notification) and a known, non-remote origin. Input from Telegram/Discord/...
/// is untrusted and its parks TTL-deny silently by design, so a remote sender
/// must not be able to raise desktop notifications by provoking an
/// `external_effect` call. The rule is [`surface::is_pet_surfaceable`], shared
/// with the inbox and the digest count.
pub(crate) fn approval_needs_surface(
    event: &DomainEvent,
    pending: &[PendingApproval],
) -> Option<DomainEvent> {
    let DomainEvent::ApprovalRequested {
        request_id,
        tool_name,
        action_summary,
        thread_id,
        ..
    } = event
    else {
        return None;
    };
    let row = pending.iter().find(|p| &p.request_id == request_id)?;
    // Thread-routed approvals already show a chat card; only a Pet companion
    // hand-off also gets the desktop notification, since its thread is one the
    // user did not open themselves.
    if thread_id.is_some() && row.origin_class.as_deref() != Some("TrustedAutomation(PetCompanion)")
    {
        return None;
    }
    if !surface::is_pet_surfaceable(row) {
        log::debug!("[pet::bus] approval not surfaced (remote, flow or unknown origin) request_id={request_id}");
        return None;
    }
    Some(DomainEvent::PetApprovalNeeded {
        request_id: request_id.clone(),
        tool_name: tool_name.clone(),
        action_summary: action_summary.clone(),
    })
}

#[async_trait]
impl EventHandler<DomainEvent> for PetApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "pet::approval_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["approval"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if !matches!(
            event,
            DomainEvent::ApprovalRequested {
                thread_id: None,
                ..
            }
        ) {
            return;
        }
        let Some(gate) = crate::neppy::security::approval::ApprovalGate::try_global() else {
            return;
        };
        let pending = match gate.list_pending() {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("[pet::bus] listing pending approvals failed: {e}");
                return;
            }
        };
        if let Some(surfaced) = approval_needs_surface(event, &pending) {
            log::info!("[pet::bus] background approval waiting — surfacing");
            BUS.publish(surfaced);
        }
    }
}
