use tempfile::TempDir;

use super::ops::pet_update;
use super::run::RunNowGuard;
use super::store;
use super::store_feed;
use super::store_tests::test_config;
use super::types::PetProfilePatch;
use crate::neppy::cron;

async fn enabled_pet(config: &crate::neppy::config::Config) -> (String, cron::CronJob) {
    let patch: PetProfilePatch =
        serde_json::from_value(serde_json::json!({ "enabled": true })).unwrap();
    let profile = pet_update(config, patch).await.unwrap().value;
    let job = cron::get_job(config, profile.research_job_id.as_deref().unwrap()).unwrap();
    (profile.id, job)
}

#[tokio::test]
async fn cron_run_of_a_pet_job_shares_the_run_now_guard() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let (pet_id, job) = enabled_pet(&config).await;

    // A `pet_run_now` pass is in flight: a manual cron execution (Automations
    // "Run now" / `cron_run`) of the same job must not start a second pass.
    let guard = RunNowGuard::try_acquire(&pet_id).unwrap().expect("free");
    let (success, output) = cron::scheduler::execute_job_now(&config, &job).await;
    assert!(!success);
    assert!(output.contains("already running"), "got {output}");
    assert!(
        store_feed::last_run(&config, &pet_id).unwrap().is_none(),
        "a rejected run must not be surfaced"
    );
    // ...and the reverse: pet_run_now sees the guard too.
    assert!(super::ops::pet_run_now(&config, true)
        .await
        .unwrap_err()
        .contains("already running"));
    drop(guard);
    assert!(RunNowGuard::try_acquire(&pet_id).unwrap().is_some());
}

#[tokio::test]
async fn cron_run_of_a_pet_job_surfaces_the_pass_afterwards() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let (pet_id, job) = enabled_pet(&config).await;
    assert!(store_feed::last_run(&config, &pet_id).unwrap().is_none());

    // The agent run itself fails in a test environment (no provider); the
    // pass is surfaced regardless, as `pet_run_now` does.
    let _ = cron::scheduler::execute_job_now(&config, &job).await;

    let run = store_feed::last_run(&config, &pet_id)
        .unwrap()
        .expect("a manual cron execution must record a pet run via surface_after_pass");
    assert_eq!(run.trigger, "manual");
    assert!(store::get_pet(&config, &pet_id)
        .unwrap()
        .unwrap()
        .last_pass_at
        .is_some());
    // The guard was released afterwards.
    assert!(RunNowGuard::try_acquire(&pet_id).unwrap().is_some());
}

#[tokio::test]
async fn scheduled_pet_tick_skips_while_a_manual_pass_runs() {
    use super::run::{begin_scheduled_tick, ScheduledTick};
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let (pet_id, job) = enabled_pet(&config).await;

    // Free: the tick proceeds and holds the per-pet guard while it runs.
    let tick = begin_scheduled_tick(&config, &job);
    let ScheduledTick::Proceed(Some(guard)) = tick else {
        panic!("a free pet must let the tick proceed with the guard");
    };
    assert!(RunNowGuard::try_acquire(&pet_id).unwrap().is_none());
    // While a tick (or a manual pass) holds it, the next tick is skipped.
    assert!(matches!(
        begin_scheduled_tick(&config, &job),
        ScheduledTick::Skip
    ));
    drop(guard);
    assert!(matches!(
        begin_scheduled_tick(&config, &job),
        ScheduledTick::Proceed(Some(_))
    ));

    // Non-pet jobs never take the guard.
    let mut other = job.clone();
    other.agent_id = Some("morning_briefing".into());
    assert!(matches!(
        begin_scheduled_tick(&config, &other),
        ScheduledTick::Proceed(None)
    ));
}
