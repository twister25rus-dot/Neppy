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
