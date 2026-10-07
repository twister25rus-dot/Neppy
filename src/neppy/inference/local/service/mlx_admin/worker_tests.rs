//! Tests for worker control, crash reaping and the worker RPC payloads.
//!
//! Every pool test uses a block id no real install has, because the pool's
//! stop/reap paths consult spawn markers under the shared `~/.neppy` root —
//! an id of `primary` there could address the user's own running server.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::metrics::MetricsSink;
use super::super::pool::MlxPool;
use super::super::process::MlxProcess;
use super::super::worker_rpc::{worker_metrics, worker_status, WorkerMetricsParams};
use super::*;

fn unique_id(tag: &str) -> String {
    format!("t1-{tag}-{}", std::process::id())
}

#[test]
fn same_model_accepts_ids_and_cache_paths() {
    let id = "ornith-ai/Ornith-1.5-9B-MLX-8bit";
    assert!(same_model(id, id));
    assert!(same_model(" ORNITH-AI/Ornith-1.5-9B-MLX-8bit ", id));
    assert!(same_model(
        "/Users/x/.cache/huggingface/hub/models--ornith-ai--Ornith-1.5-9B-MLX-8bit/snapshots/abc",
        id
    ));
    assert!(same_model("/models/ornith-ai/Ornith-1.5-9B-MLX-8bit", id));
    assert!(!same_model("LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit", id));
    assert!(!same_model("ornith-ai/Ornith-1.5-9B-MLX-4bit", id));
}

#[test]
fn restart_budget_slides_over_ten_minutes() {
    let control = WorkerControl::default();
    let t0 = Instant::now();
    assert!(control.try_restart(t0, 3));
    assert!(control.try_restart(t0 + Duration::from_secs(1), 3));
    assert!(control.try_restart(t0 + Duration::from_secs(2), 3));
    assert!(
        !control.try_restart(t0 + Duration::from_secs(3), 3),
        "fourth in window"
    );
    // The first restart ages out after ten minutes.
    assert!(control.try_restart(t0 + Duration::from_secs(600), 3));
    assert!(
        !control.try_restart(t0 + Duration::from_secs(600), 3),
        "window full again"
    );
    // The second ages out a second later.
    assert!(control.try_restart(t0 + Duration::from_secs(601), 3));
    assert!(
        !WorkerControl::default().try_restart(t0, 0),
        "zero allows none"
    );
}

#[test]
fn worker_id_prefers_primary() {
    let mut config = Config::default();
    assert_eq!(worker_server_id(&config).as_deref(), Some("primary"));
    config.mlx.servers[0].id = "other".into();
    assert_eq!(worker_server_id(&config).as_deref(), Some("other"));
    config.mlx.servers.clear();
    assert_eq!(worker_server_id(&config), None);
}

#[test]
fn ollama_ps_is_parsed_leniently() {
    let body = r#"{"models":[{"name":"bge-m3:latest","size":1258291200,"extra":1},{"name":""}]}"#;
    assert_eq!(
        parse_ollama_ps(body),
        vec![OllamaLoaded {
            name: "bge-m3:latest".into(),
            size_mib: 1200
        }]
    );
    assert!(parse_ollama_ps("{}").is_empty());
    assert!(parse_ollama_ps("not json").is_empty());
}

#[tokio::test]
async fn a_crashed_child_is_reaped_and_recorded() {
    let id = unique_id("reap");
    let config = Config::default();
    let metrics = Arc::new(MetricsSink::new());
    let pool = MlxPool::with_metrics(Arc::clone(&metrics));

    let child = tokio::process::Command::new("sh")
        .args(["-c", "exit 3"])
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sh");
    pool.insert_for_test(MlxProcess::from_child_for_test(&id, 1, child))
        .await;
    assert!(pool.pid_of(&id).await.is_some());

    // Wait for the child to exit.
    for _ in 0..100 {
        if pool.process_of(&id).await.is_some_and(|p| !p.alive) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!pool.is_running(&id).await);

    let reaped = pool.reap_crashed(&config).await;
    assert_eq!(reaped, vec![id.clone()]);
    assert!(pool.process_of(&id).await.is_none(), "the entry is gone");
    assert!(
        pool.reap_crashed(&config).await.is_empty(),
        "reaping is idempotent"
    );

    let events = metrics.recent(0, 10, true).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "worker_crash");
    assert!(events[0].detail.contains("code 3"), "{}", events[0].detail);
}

#[tokio::test]
async fn a_live_child_is_not_reaped_and_stop_records_it() {
    let id = unique_id("live");
    let config = Config::default();
    let metrics = Arc::new(MetricsSink::new());
    let pool = MlxPool::with_metrics(Arc::clone(&metrics));

    let child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sleep");
    pool.insert_for_test(MlxProcess::from_child_for_test(&id, 1, child))
        .await;
    assert!(pool.is_running(&id).await);
    assert!(pool.reap_crashed(&config).await.is_empty());

    assert!(pool.stop(&config, &id).await);
    assert!(pool.process_of(&id).await.is_none());
    let events = metrics.recent(0, 10, true).events;
    assert_eq!(events.last().map(|e| e.event.as_str()), Some("worker_stop"));
}

#[tokio::test]
async fn ensure_started_is_a_no_op_when_the_supervisor_is_off() {
    let config = Config::default();
    assert!(!config.mlx.enabled);
    let svc = Arc::new(LocalAiService::new(&config));
    ensure_started(&svc, &config).await.expect("no-op");
    assert!(svc.metrics.recent(0, 10, false).events.is_empty());
}

#[tokio::test]
async fn worker_status_reports_an_idle_unsupervised_worker() {
    let config = Config::default();
    let svc = Arc::new(LocalAiService::new(&config));
    let status = worker_status(&svc, &config).await;
    assert!(!status.supervised);
    assert_eq!(status.server_id.as_deref(), Some("primary"));
    assert!(status.worker.is_none());
    assert_eq!(status.pressure_state, "normal");
    assert_eq!(status.gate.active, 0);
    assert_eq!(status.policy.max_waiters, 4);
    let json = serde_json::to_value(&status).expect("serializes");
    assert!(json["system"]["total_bytes"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn worker_metrics_clamps_the_limit() {
    let config = Config::default();
    let svc = LocalAiService::new(&config);
    for ts in 0..3000 {
        svc.metrics
            .record_sample(super::super::metrics::MetricsSample {
                ts_ms: ts,
                ..Default::default()
            });
    }
    let all = worker_metrics(
        &svc,
        &WorkerMetricsParams {
            limit: Some(1_000_000),
            ..Default::default()
        },
    );
    assert_eq!(all.samples.len(), super::super::metrics::SAMPLE_RING_CAP);
    let default = worker_metrics(&svc, &WorkerMetricsParams::default());
    assert_eq!(default.samples.len(), 200);
    let one = worker_metrics(
        &svc,
        &WorkerMetricsParams {
            limit: Some(0),
            events_only: Some(true),
            ..Default::default()
        },
    );
    assert!(one.samples.is_empty());
}

#[tokio::test]
async fn gpu_and_ollama_probes_are_cached_and_test_safe() {
    let control = WorkerControl::default();
    let http = reqwest::Client::new();
    // Unit tests never reach a real Ollama daemon.
    assert!(control.ollama_loaded(&http).await.is_empty());
    let first = control.gpu_in_use().await;
    let second = control.gpu_in_use().await;
    assert_eq!(
        first, second,
        "a second read within 10s is the cached value"
    );
    assert_eq!(control.restarts_in_window(), 0);
}

// ---- never unload a server this process does not hold --------------------

/// A stand-in for `mlx_vlm.server`: `/health` reports `loaded`, `/unload`
/// succeeds and is counted. Returns its port and the unload counter.
async fn mock_worker(loaded: &str) -> (u16, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let unloads = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&unloads);
    let health = format!(
        r#"{{"status":"healthy","loaded_model":"{loaded}","loaded_models":{{"language":"{loaded}"}}}}"#
    );
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (counter, health) = (Arc::clone(&counter), health.clone());
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let body = if head.starts_with("POST /unload") {
                    counter.fetch_add(1, Ordering::SeqCst);
                    "{}".to_string()
                } else if head.starts_with("GET /health") {
                    health
                } else {
                    "{}".to_string()
                };
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    (port, unloads)
}

fn config_for_port(port: u16) -> Config {
    let mut config = Config::default();
    config.mlx.servers[0].port = port;
    config
}

#[tokio::test]
async fn a_model_switch_never_unloads_a_server_this_process_does_not_hold() {
    use std::sync::atomic::Ordering;
    let (port, unloads) = mock_worker("someone/elses-model").await;
    let config = config_for_port(port);
    let svc = Arc::new(LocalAiService::new(&config));

    let err = prepare_model(&svc, &config, "wanted/model")
        .await
        .expect_err("another process's server must not be unloaded");
    assert!(err.contains("was not started by Neppy"), "{err}");
    assert!(err.contains("someone/elses-model"), "{err}");
    assert_eq!(unloads.load(Ordering::SeqCst), 0, "no /unload was sent");
}

#[tokio::test]
async fn an_unsupervised_server_that_already_has_the_wanted_model_is_used_as_is() {
    use std::sync::atomic::Ordering;
    let (port, unloads) = mock_worker("wanted/model").await;
    let config = config_for_port(port);
    let svc = Arc::new(LocalAiService::new(&config));
    assert_eq!(
        prepare_model(&svc, &config, "wanted/model").await,
        Ok(false)
    );
    assert_eq!(unloads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_held_worker_still_switches_models_by_unloading() {
    use std::sync::atomic::Ordering;
    let (port, unloads) = mock_worker("old/model").await;
    let config = config_for_port(port);
    let svc = Arc::new(LocalAiService::new(&config));
    let child = tokio::process::Command::new("sleep")
        .arg("30")
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sleep");
    svc.mlx
        .insert_for_test(MlxProcess::from_child_for_test("primary", port, child))
        .await;
    assert_eq!(prepare_model(&svc, &config, "wanted/model").await, Ok(true));
    assert_eq!(unloads.load(Ordering::SeqCst), 1);
}

// ── "off" is a persisted user intent ─────────────────────────────────────
//
// Hermetic: every test here is refused before anything spawns, so no real
// MLX process or marker is ever touched.

fn stopped_by_user_config(id: &str) -> Config {
    let mut config = Config::default();
    config.mlx.enabled = true;
    config.mlx.servers[0].id = id.to_string();
    config.mlx.servers[0].user_stopped = true;
    // Belt and braces: even a regression cannot find a real binary here.
    config.mlx.bin_dir = "/nonexistent-neppy-test-bin-dir".to_string();
    config
}

#[tokio::test]
async fn a_user_stopped_server_is_not_lazily_started_by_a_request() {
    let id = unique_id("user-stopped-lazy");
    let config = stopped_by_user_config(&id);
    let svc = Arc::new(LocalAiService::new(&config));

    let err = ensure_started(&svc, &config)
        .await
        .expect_err("an inference request must not start a server the user turned off");
    assert!(err.contains("turned off"), "{err}");
    assert!(err.contains(&id), "{err}");
    assert!(svc.mlx.process_of(&id).await.is_none(), "nothing spawned");
    assert!(
        svc.mlx.base_url_for(&id).await.is_none(),
        "nothing registered in the pool"
    );
    let events = svc.metrics.recent(0, 10, true).events;
    assert!(
        events.iter().any(|e| e.event == "admission_refused"),
        "the refusal is recorded: {events:?}"
    );
}

#[tokio::test]
async fn a_user_stopped_server_that_is_reachable_elsewhere_is_still_used_as_is() {
    // Not ours and answering: the existing "use what is there" rule is
    // untouched, because nothing is being started.
    let (port, _unloads) = mock_worker("someone/elses-model").await;
    let id = "primary".to_string();
    let mut config = config_for_port(port);
    config.mlx.enabled = true;
    config.mlx.servers[0].user_stopped = true;
    let svc = Arc::new(LocalAiService::new(&config));
    ensure_started(&svc, &config)
        .await
        .expect("reachable server is used, nothing started");
    assert!(svc.mlx.process_of(&id).await.is_none());
}

#[tokio::test]
async fn the_refusal_helper_only_fires_for_a_stopped_block() {
    let id = unique_id("refusal-helper");
    let stopped = stopped_by_user_config(&id);
    assert!(refuse_if_user_stopped(&stopped, &id).await.is_some());

    let mut running = stopped.clone();
    running.mlx.servers[0].user_stopped = false;
    assert!(refuse_if_user_stopped(&running, &id).await.is_none());
    assert!(
        refuse_if_user_stopped(&stopped, "no-such-block")
            .await
            .is_none(),
        "an unknown block is not 'turned off'"
    );
}

#[tokio::test]
async fn boot_autostart_skips_a_user_stopped_block() {
    let id = unique_id("reconcile-stopped");
    let config = stopped_by_user_config(&id);
    assert!(
        config.mlx.servers[0].autostart,
        "autostart alone would start it"
    );
    let pool = MlxPool::new();
    pool.reconcile(&config, &reqwest::Client::new()).await;
    assert!(pool.process_of(&id).await.is_none());
    assert!(pool.base_url_for(&id).await.is_none());
}

#[test]
fn an_explicit_start_clears_the_flag_and_stop_sets_it() {
    use crate::neppy::inference::local::mlx::{apply_user_stopped, is_user_stopped};
    let mut config = Config::default();
    assert!(!is_user_stopped(&config, "primary"), "default is not off");

    assert!(
        apply_user_stopped(&mut config, "primary", true),
        "stop sets it"
    );
    assert!(is_user_stopped(&config, "primary"));
    assert!(
        !apply_user_stopped(&mut config, "primary", true),
        "idempotent"
    );

    assert!(
        apply_user_stopped(&mut config, "primary", false),
        "start clears it"
    );
    assert!(!is_user_stopped(&config, "primary"));
    assert!(
        !apply_user_stopped(&mut config, "missing", true),
        "unknown id"
    );
}

#[test]
fn a_config_without_the_new_field_loads_as_not_turned_off() {
    let server: crate::neppy::config::schema::MlxServerConfig =
        toml::from_str("id = \"primary\"\nautostart = true\n").expect("old block parses");
    assert!(!server.user_stopped);
    let round: crate::neppy::config::schema::MlxServerConfig =
        toml::from_str(&toml::to_string(&server).unwrap()).unwrap();
    assert_eq!(round.user_stopped, server.user_stopped);
}
