//! `decide()` table and watchdog wiring tests.

use std::time::Duration;

use super::*;

use PressureState::{Critical as C, Elevated as E, Normal as N};
use WorkerAction as A;
use WorkerPhase as W;

fn inputs(pressure: PressureState, idle_secs: u64, worker: WorkerPhase) -> WatchInputs {
    WatchInputs {
        pressure,
        idle_for: Duration::from_secs(idle_secs),
        worker,
        gate_active: false,
        gate_paused: false,
        server_busy: false,
        pressure_only: false,
        idle_unload_secs: 300,
        idle_stop_secs: 900,
    }
}

#[test]
fn decide_table() {
    let active = |mut i: WatchInputs| {
        i.gate_active = true;
        i
    };
    let paused = |mut i: WatchInputs| {
        i.gate_paused = true;
        i
    };
    let busy = |mut i: WatchInputs| {
        i.server_busy = true;
        i
    };
    let pressure_only = |mut i: WatchInputs| {
        i.pressure_only = true;
        i
    };

    let cases: Vec<(&str, WatchInputs, WorkerAction)> = vec![
        ("fresh and loaded", inputs(N, 10, W::Loaded), A::None),
        (
            "idle 299s keeps the model",
            inputs(N, 299, W::Loaded),
            A::None,
        ),
        ("idle 300s unloads", inputs(N, 300, W::Loaded), A::Unload),
        (
            "idle 300s already unloaded",
            inputs(N, 300, W::Unloaded),
            A::None,
        ),
        ("idle 900s stops", inputs(N, 900, W::Loaded), A::Stop),
        (
            "idle 900s unloaded also stops",
            inputs(N, 900, W::Unloaded),
            A::Stop,
        ),
        ("elevated and idle stops", inputs(E, 1, W::Loaded), A::Stop),
        (
            "elevated while active waits for the step",
            active(inputs(E, 0, W::Loaded)),
            A::None,
        ),
        ("critical stops now", inputs(C, 0, W::Loaded), A::StopNow),
        (
            "critical stops now even while active",
            active(inputs(C, 0, W::Loaded)),
            A::StopNow,
        ),
        (
            "critical while starting stops now",
            inputs(C, 0, W::Starting),
            A::StopNow,
        ),
        (
            "crashed is reaped",
            inputs(N, 0, W::Crashed),
            A::ReapCrashed,
        ),
        (
            "crashed is reaped under pressure too",
            inputs(C, 0, W::Crashed),
            A::ReapCrashed,
        ),
        (
            "pressure_only keeps it resident",
            pressure_only(inputs(N, 5000, W::Loaded)),
            A::None,
        ),
        (
            "pressure_only still stops on elevated",
            pressure_only(inputs(E, 0, W::Loaded)),
            A::Stop,
        ),
        (
            "active requests are never idled out",
            active(inputs(N, 5000, W::Loaded)),
            A::None,
        ),
        ("stopped and normal", inputs(N, 5000, W::Stopped), A::None),
        ("stopped under pressure", inputs(E, 0, W::Stopped), A::None),
        (
            "recovered pause resumes",
            paused(inputs(N, 0, W::Stopped)),
            A::Resume,
        ),
        (
            "still-elevated pause holds",
            paused(inputs(E, 0, W::Stopped)),
            A::None,
        ),
        (
            "starting is not idled",
            inputs(N, 400, W::Starting),
            A::None,
        ),
        (
            "idle 300s but server busy defers unload",
            busy(inputs(N, 300, W::Loaded)),
            A::None,
        ),
        (
            "idle 900s but server busy defers stop",
            busy(inputs(N, 900, W::Loaded)),
            A::None,
        ),
        (
            "elevated but server busy defers stop",
            busy(inputs(E, 0, W::Loaded)),
            A::None,
        ),
        (
            "critical stops now even when busy",
            busy(inputs(C, 0, W::Loaded)),
            A::StopNow,
        ),
        (
            "busy crashed worker is still reaped",
            busy(inputs(N, 0, W::Crashed)),
            A::ReapCrashed,
        ),
        (
            "busy does not block resume",
            busy(paused(inputs(N, 0, W::Loaded))),
            A::Resume,
        ),
    ];

    for (name, input, expected) in cases {
        assert_eq!(decide(&input), expected, "case: {name}");
    }
}

#[test]
fn zero_timers_disable_idle_actions() {
    let mut i = inputs(N, 100_000, W::Loaded);
    i.idle_unload_secs = 0;
    i.idle_stop_secs = 0;
    assert_eq!(decide(&i), A::None);
}

#[test]
fn phase_names_are_stable() {
    for (phase, name) in [
        (W::Stopped, "stopped"),
        (W::Starting, "starting"),
        (W::Loaded, "loaded"),
        (W::Unloaded, "unloaded"),
        (W::Crashed, "crashed"),
    ] {
        assert_eq!(phase.as_str(), name);
    }
}

#[tokio::test]
async fn a_disabled_supervisor_ticks_without_touching_anything() {
    let config = crate::neppy::config::Config::default();
    assert!(!config.mlx.enabled);
    let svc = std::sync::Arc::new(crate::neppy::inference::local::LocalAiService::new(&config));
    assert_eq!(tick(&svc, &config).await, Duration::from_secs(30));
    assert!(svc.metrics.latest_sample().is_none());
}

#[tokio::test]
async fn an_enabled_idle_supervisor_records_a_sample() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = crate::neppy::config::Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config.mlx.enabled = true;
    // A server id no real install has, so no path can address a live server.
    config.mlx.servers[0].id = format!("test-tick-{}", std::process::id());
    let svc = std::sync::Arc::new(crate::neppy::inference::local::LocalAiService::new(&config));
    let pause = tick(&svc, &config).await;
    let sample = svc.metrics.latest_sample().expect("a sample");
    assert_eq!(sample.worker_state, "stopped");
    assert!(sample.worker_pid.is_none());
    assert!(sample.avail_pct > 0.0);
    assert!(pause >= Duration::from_secs(10));
    assert!(dir.path().join("local_assistant").join("metrics").is_dir());
}

#[test]
fn server_busy_reads_queue_and_in_flight() {
    use super::super::health::{parse_in_flight, server_busy};
    assert!(!server_busy(None, None), "a failed probe is not busy");
    assert!(!server_busy(Some(0), Some(0)));
    assert!(server_busy(Some(1), None));
    assert!(server_busy(None, Some(2)));

    let metrics = serde_json::json!({"latest": null, "summary": {"in_flight": 3}});
    assert_eq!(parse_in_flight(&metrics), Some(3));
    assert_eq!(
        parse_in_flight(&serde_json::json!({"in_flight": 1})),
        Some(1)
    );
    assert_eq!(parse_in_flight(&serde_json::json!({"summary": {}})), None);
}

#[tokio::test]
async fn an_unreachable_server_probes_as_not_busy() {
    // Port 9 (discard) on loopback: nothing answers, so the probe fails fast.
    let http = reqwest::Client::new();
    let in_flight =
        super::super::health::probe_in_flight(&http, "http://127.0.0.1:9/v1", None).await;
    assert_eq!(in_flight, None);
}

#[test]
fn health_reports_queue_depth() {
    let live: super::super::health::LivenessReport =
        serde_json::from_value(serde_json::json!({"loaded_model": "m", "request_queue_depth": 2}))
            .expect("parses");
    assert_eq!(live.request_queue_depth, Some(2));
}

#[test]
fn an_unload_that_frees_nothing_is_ineffective() {
    const GIB: u64 = 1024 * 1024 * 1024;
    // Measured on mlx_vlm 0.7.0 with a 9 GiB model: 9.9 GiB before, 9.4 GiB after.
    assert!(!unload_released_memory(
        10_400 * 1024 * 1024,
        9_830 * 1024 * 1024
    ));
    // Weights actually freed.
    assert!(unload_released_memory(10 * GIB, 600 * 1024 * 1024));
    // Exactly half counts as freed; a hair under does not.
    assert!(unload_released_memory(10 * GIB, 5 * GIB));
    assert!(!unload_released_memory(10 * GIB, 5 * GIB + 1));
    // A small worker is never judged: there is nothing worth escalating for.
    assert!(unload_released_memory(512 * 1024 * 1024, 512 * 1024 * 1024));
}

// ---- Ollama is only touched on Critical ----------------------------------

fn transition(from: PressureState, to: PressureState) -> Transition {
    Transition {
        from,
        to,
        reason: "test".into(),
    }
}

#[test]
fn ollama_models_are_dropped_only_on_entering_critical() {
    let table = [
        (N, E, false), // the first Elevated used to unload them
        (E, N, false),
        (N, C, true),
        (E, C, true),
        (C, E, false),
        (C, N, false),
    ];
    for (from, to, expect) in table {
        assert_eq!(
            ollama_unload_due(&transition(from, to)),
            expect,
            "{from:?} -> {to:?}"
        );
    }
}

// ---- Stop and Unload do not race a request -------------------------------

use super::super::process::MlxProcess;

async fn svc_with_worker(tag: &str) -> (Arc<LocalAiService>, Config, String) {
    let config = Config::default();
    let svc = Arc::new(LocalAiService::new(&config));
    let id = format!("t2-{tag}-{}", std::process::id());
    let child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sleep");
    svc.mlx
        .insert_for_test(MlxProcess::from_child_for_test(&id, 1, child))
        .await;
    (svc, config, id)
}

fn gate_cfg() -> crate::neppy::config::schema::MlxWorkerConfig {
    crate::neppy::config::schema::MlxWorkerConfig::default()
}

#[tokio::test]
async fn an_elevated_stop_is_skipped_while_a_request_holds_the_gate() {
    let (svc, config, id) = svc_with_worker("busy").await;
    let request = svc.gate.acquire(&gate_cfg()).await.expect("request");
    apply(
        &svc,
        &config,
        &id,
        WorkerAction::Stop,
        PressureState::Elevated,
    )
    .await;
    assert!(
        svc.mlx.is_running(&id).await,
        "the worker must survive: a request was admitted after the verdict"
    );
    drop(request);
    apply(
        &svc,
        &config,
        &id,
        WorkerAction::Stop,
        PressureState::Elevated,
    )
    .await;
    assert!(svc.mlx.process_of(&id).await.is_none(), "stopped once idle");
}

#[tokio::test]
async fn an_idle_unload_is_skipped_while_a_request_holds_the_gate() {
    let (svc, config, id) = svc_with_worker("unload").await;
    let request = svc.gate.acquire(&gate_cfg()).await.expect("request");
    apply(
        &svc,
        &config,
        &id,
        WorkerAction::Unload,
        PressureState::Normal,
    )
    .await;
    assert!(svc.mlx.is_running(&id).await);
    assert!(
        svc.metrics.recent(0, 10, true).events.is_empty(),
        "no unload event: nothing was unloaded"
    );
    drop(request);
    assert!(svc.mlx.stop_if_held(&config, &id).await);
}

#[tokio::test]
async fn a_critical_stop_cancels_the_request_first_and_does_not_wait() {
    let (svc, config, id) = svc_with_worker("critical").await;
    let request = svc.gate.acquire(&gate_cfg()).await.expect("request");
    apply(
        &svc,
        &config,
        &id,
        WorkerAction::StopNow,
        PressureState::Critical,
    )
    .await;
    assert!(
        request.is_cancelled(),
        "the in-flight request was preempted"
    );
    assert!(
        svc.mlx.process_of(&id).await.is_none(),
        "and the worker stopped"
    );
}

#[tokio::test]
async fn the_watchdog_stops_only_a_worker_it_holds() {
    let config = Config::default();
    let svc = Arc::new(LocalAiService::new(&config));
    let id = format!("t2-unheld-{}", std::process::id());
    // No handle: `stop` would fall back to the spawn marker; the watchdog
    // must not.
    assert!(!svc.mlx.stop_if_held(&config, &id).await);
    apply(
        &svc,
        &config,
        &id,
        WorkerAction::StopNow,
        PressureState::Critical,
    )
    .await;
    apply(
        &svc,
        &config,
        &id,
        WorkerAction::Stop,
        PressureState::Elevated,
    )
    .await;
}
