//! Manual Pet passes: `pet_run_now` and any other manual execution of the
//! research job (Automations "Run now", `cron_run`).
//!
//! Both entry points share ONE in-flight guard per pet and both surface the
//! pass afterwards (rank notes, digest, notifications), so a manual run can
//! never double-run a pass or leave its notes unranked.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use chrono::Utc;

use crate::neppy::config::Config;
use crate::neppy::cron::{self, CronJob};
use crate::rpc::RpcOutcome;

use super::ops::ensure_research_job;
use super::store;
use super::surface;
use super::types::PetRunSummary;

const ALREADY_RUNNING: &str = "a Pet pass is already running";

fn run_now_in_flight() -> &'static Mutex<HashSet<String>> {
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Holds a pet's in-flight slot; released on drop (including cancellation).
pub(crate) struct RunNowGuard(String);

impl RunNowGuard {
    /// `None` when a manual pass for `pet_id` is already running.
    pub(crate) fn try_acquire(pet_id: &str) -> Result<Option<Self>, &'static str> {
        let mut set = run_now_in_flight()
            .lock()
            .map_err(|_| "run-now lock poisoned")?;
        Ok(set
            .insert(pet_id.to_string())
            .then(|| Self(pet_id.to_string())))
    }
}

impl Drop for RunNowGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = run_now_in_flight().lock() {
            set.remove(&self.0);
        }
    }
}

async fn run_pass(
    config: Config,
    pet_id: String,
    job: CronJob,
    _guard: RunNowGuard,
) -> Result<PetRunSummary> {
    let started_at = Utc::now();
    let (success, output) = cron::scheduler::execute_job_now_unrouted(&config, &job).await;
    let finished_at = Utc::now();
    let status = if success { "ok" } else { "error" };
    let _ = cron::record_run(
        &config,
        &job.id,
        started_at,
        finished_at,
        status,
        Some(&output),
        (finished_at - started_at).num_milliseconds(),
    );
    let _ = cron::record_last_run(&config, &job.id, finished_at, success, &output);
    log::info!("[pet] manual pass finished success={success}");
    surface::surface_after_pass(&config, &pet_id, Some(&job.id), "manual", success).await
}

pub async fn pet_run_now(config: &Config, wait: bool) -> Result<RpcOutcome<PetRunSummary>, String> {
    let pet = store::ensure_primary(config, Utc::now()).map_err(|e| e.to_string())?;
    let Some(guard) = RunNowGuard::try_acquire(&pet.id)? else {
        return Err(ALREADY_RUNNING.into());
    };
    let job = ensure_research_job(config, &pet).map_err(|e| e.to_string())?;
    log::info!("[pet] run_now pet_id={} wait={wait}", pet.id);
    let fut = run_pass(config.clone(), pet.id.clone(), job, guard);
    if wait {
        return Ok(RpcOutcome::new(
            fut.await.map_err(|e| e.to_string())?,
            vec![],
        ));
    }
    tokio::spawn(async move {
        if let Err(e) = fut.await {
            log::warn!("[pet] background pass failed: {e}");
        }
    });
    Ok(RpcOutcome::new(PetRunSummary::started("manual"), vec![]))
}

/// A manual execution of `job` that did not come through [`pet_run_now`]
/// (Automations "Run now", the `cron_run` RPC or tool). Takes the same
/// in-flight guard as `pet_run_now`, runs the job, and surfaces the pass
/// afterwards. The caller records the run in the cron history as usual.
/// Returns `(success, output)` like `execute_job_now`.
pub async fn execute_manual_job(config: &Config, job: &CronJob) -> (bool, String) {
    let pet_id = match store::find_pet_by_job(config, &job.id) {
        Ok(Some(id)) => Some(id),
        Ok(None) => {
            log::warn!(
                "[pet] manual run of an unowned research job job_id={}",
                job.id
            );
            None
        }
        Err(e) => {
            log::warn!("[pet] pet lookup failed for manual run: {e}");
            None
        }
    };
    let Some(pet_id) = pet_id else {
        // No pet to surface for: still the read-only lane, run it plainly.
        return cron::scheduler::execute_job_now_unrouted(config, job).await;
    };
    let guard = match RunNowGuard::try_acquire(&pet_id) {
        Ok(Some(g)) => g,
        Ok(None) => {
            log::debug!("[pet] manual run rejected: a pass is already running pet_id={pet_id}");
            return (false, ALREADY_RUNNING.to_string());
        }
        Err(e) => return (false, e.to_string()),
    };
    let (success, output) = cron::scheduler::execute_job_now_unrouted(config, job).await;
    log::info!("[pet] manual (cron) pass finished success={success}");
    if let Err(e) =
        surface::surface_after_pass(config, &pet_id, Some(&job.id), "manual", success).await
    {
        log::warn!("[pet] surfacing after manual cron run failed: {e}");
    }
    drop(guard);
    (success, output)
}
