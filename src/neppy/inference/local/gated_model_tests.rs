//! Tests for `GatedLocalModel`: the permit is released on every path, at most
//! one request is active, preemption cancels, and the factory wraps exactly
//! the `mlx:` branch.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use tinyagents::harness::model::{
    ChatModel, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinyagents::harness::usage::Usage;
use tinyagents::TinyAgentsError;

use super::*;

/// What the scripted inner model does on each call.
#[derive(Clone, Copy)]
enum Script {
    Ok,
    Fail,
    /// Never answers until cancelled.
    Hang,
}

struct ScriptedModel {
    script: Script,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    calls: AtomicUsize,
}

impl ScriptedModel {
    fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            script,
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ChatModel<()> for ScriptedModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyagents::Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let result = match self.script {
            Script::Ok => {
                tokio::time::sleep(Duration::from_millis(2)).await;
                Ok(ModelResponse::assistant("hi").with_usage(Usage {
                    input_tokens: 11,
                    output_tokens: 7,
                    ..Usage::default()
                }))
            }
            Script::Fail => Err(TinyAgentsError::Model("inner failure".into())),
            Script::Hang => {
                futures::future::pending::<()>().await;
                unreachable!()
            }
        };
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        result
    }

    async fn stream(&self, _state: &(), _request: ModelRequest) -> tinyagents::Result<ModelStream> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let head = futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::UsageDelta(Usage {
                input_tokens: 5,
                output_tokens: 3,
                ..Usage::default()
            }),
        ]);
        let tail: ModelStream = match self.script {
            Script::Ok => Box::pin(futures::stream::iter(vec![ModelStreamItem::Completed(
                ModelResponse::assistant("done"),
            )])),
            Script::Fail => Box::pin(futures::stream::iter(vec![ModelStreamItem::Failed(
                "boom".into(),
            )])),
            // Items after the head never arrive.
            Script::Hang => Box::pin(futures::stream::pending()),
        };
        Ok(Box::pin(head.chain(tail)))
    }
}

struct FakeWorker {
    calls: AtomicUsize,
    fail: bool,
    loading: bool,
}

#[async_trait]
impl WorkerReady for FakeWorker {
    async fn prepare(&self, _model_id: &str) -> Result<bool, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err("admission refused".into())
        } else {
            Ok(self.loading)
        }
    }
}

struct Rig {
    model: GatedLocalModel,
    inner: Arc<ScriptedModel>,
    gate: Arc<InferenceGate>,
    metrics: Arc<MetricsSink>,
    worker: Arc<FakeWorker>,
}

fn rig(script: Script, fail_prepare: bool) -> Rig {
    let inner = ScriptedModel::new(script);
    let gate = Arc::new(InferenceGate::new());
    let metrics = Arc::new(MetricsSink::new());
    let worker = Arc::new(FakeWorker {
        calls: AtomicUsize::new(0),
        fail: fail_prepare,
        loading: true,
    });
    let cfg = MlxWorkerConfig {
        max_waiters: 64,
        acquire_timeout_secs: 30,
        ..MlxWorkerConfig::default()
    };
    let model = GatedLocalModel::new(
        Arc::clone(&inner) as Arc<dyn ChatModel<()>>,
        "org/model",
        Arc::clone(&gate),
        Arc::clone(&metrics),
        Arc::clone(&worker) as Arc<dyn WorkerReady>,
        cfg,
    );
    Rig {
        model,
        inner,
        gate,
        metrics,
        worker,
    }
}

fn events(metrics: &MetricsSink) -> Vec<String> {
    metrics
        .recent(0, 100, true)
        .events
        .into_iter()
        .map(|e| e.event)
        .collect()
}

#[tokio::test]
async fn invoke_success_releases_the_permit_and_records_usage() {
    let r = rig(Script::Ok, false);
    let response = r
        .model
        .invoke(&(), ModelRequest::default())
        .await
        .expect("ok");
    assert_eq!(response.text(), "hi");
    assert_eq!(r.gate.snapshot().active, 0);
    assert_eq!(r.worker.calls.load(Ordering::SeqCst), 1);
    let usage = r.metrics.last_usage();
    assert_eq!(
        (usage.prompt_tokens, usage.completion_tokens),
        (Some(11), Some(7))
    );
    assert_eq!(events(&r.metrics), vec!["model_ready"]);
}

#[tokio::test]
async fn invoke_error_releases_the_permit() {
    let r = rig(Script::Fail, false);
    let err = r
        .model
        .invoke(&(), ModelRequest::default())
        .await
        .expect_err("fails");
    assert!(err.to_string().contains("inner failure"));
    assert!(gate_error_of(&err).is_none());
    assert_eq!(r.gate.snapshot().active, 0);
    assert!(events(&r.metrics).is_empty(), "no model_ready on failure");
}

#[tokio::test]
async fn a_worker_that_cannot_start_fails_the_call_and_frees_the_slot() {
    let r = rig(Script::Ok, true);
    let err = r
        .model
        .invoke(&(), ModelRequest::default())
        .await
        .expect_err("fails");
    assert!(err.to_string().contains("admission refused"));
    assert_eq!(
        r.inner.calls.load(Ordering::SeqCst),
        0,
        "inner never called"
    );
    assert_eq!(r.gate.snapshot().active, 0);
    assert!(r.model.stream(&(), ModelRequest::default()).await.is_err());
    assert_eq!(r.gate.snapshot().active, 0);
}

#[tokio::test]
async fn preempt_cancels_an_in_flight_invoke() {
    let r = Arc::new(rig(Script::Hang, false));
    let call = {
        let r = Arc::clone(&r);
        tokio::spawn(async move { r.model.invoke(&(), ModelRequest::default()).await })
    };
    for _ in 0..100 {
        if r.gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(r.gate.snapshot().active, 1);
    r.gate.preempt();
    let err = tokio::time::timeout(Duration::from_secs(2), call)
        .await
        .expect("returns promptly")
        .expect("task")
        .expect_err("preempted");
    assert_eq!(gate_error_of(&err), Some(GateError::Preempted));
    assert_eq!(r.gate.snapshot().active, 0);
    assert!(events(&r.metrics).contains(&"request_preempted".to_string()));
}

#[tokio::test]
async fn a_completed_stream_releases_the_permit_at_its_terminal_item() {
    let r = rig(Script::Ok, false);
    let mut stream = r
        .model
        .stream(&(), ModelRequest::default())
        .await
        .expect("stream");
    assert_eq!(r.gate.snapshot().active, 1, "held while streaming");
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        let terminal = matches!(item, ModelStreamItem::Completed(_));
        items.push(item);
        if terminal {
            assert_eq!(r.gate.snapshot().active, 0, "released at the terminal item");
        }
    }
    assert_eq!(items.len(), 3);
    // Usage from the delta is kept when the final response carries none.
    assert_eq!(r.metrics.last_usage().prompt_tokens, Some(5));
    assert_eq!(events(&r.metrics), vec!["model_ready"]);
    drop(stream);
    assert_eq!(r.gate.snapshot().active, 0);
}

#[tokio::test]
async fn a_failed_stream_releases_the_permit() {
    let r = rig(Script::Fail, false);
    let stream = r
        .model
        .stream(&(), ModelRequest::default())
        .await
        .expect("stream");
    let items: Vec<_> = stream.collect().await;
    assert!(matches!(items.last(), Some(ModelStreamItem::Failed(_))));
    assert_eq!(r.gate.snapshot().active, 0);
}

#[tokio::test]
async fn dropping_a_stream_mid_way_releases_the_permit() {
    let r = rig(Script::Hang, false);
    let mut stream = r
        .model
        .stream(&(), ModelRequest::default())
        .await
        .expect("stream");
    assert!(matches!(
        stream.next().await,
        Some(ModelStreamItem::Started)
    ));
    assert_eq!(r.gate.snapshot().active, 1);
    drop(stream);
    assert_eq!(r.gate.snapshot().active, 0);
    // And the slot is usable again.
    let _again = r
        .model
        .stream(&(), ModelRequest::default())
        .await
        .expect("stream");
}

#[tokio::test]
async fn preempt_ends_a_stream_with_a_failure() {
    let r = rig(Script::Hang, false);
    let mut stream = r
        .model
        .stream(&(), ModelRequest::default())
        .await
        .expect("stream");
    let _ = stream.next().await;
    let _ = stream.next().await;
    r.gate.preempt();
    match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        Ok(Some(ModelStreamItem::Failed(message))) => {
            assert_eq!(
                GateError::from_message(&message),
                Some(GateError::Preempted)
            );
        }
        other => panic!(
            "expected a preemption failure, got {:?}",
            other.map(|o| o.is_some())
        ),
    }
    assert!(
        stream.next().await.is_none(),
        "the stream ends after preemption"
    );
    assert_eq!(r.gate.snapshot().active, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_never_exceeds_one_across_fifty_callers() {
    let r = Arc::new(rig(Script::Ok, false));
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let r = Arc::clone(&r);
        tasks.push(tokio::spawn(async move {
            r.model
                .invoke(&(), ModelRequest::default())
                .await
                .map(|_| ())
        }));
    }
    for task in tasks {
        task.await.expect("task").expect("invoke");
    }
    assert_eq!(r.inner.peak.load(Ordering::SeqCst), 1);
    assert_eq!(r.inner.calls.load(Ordering::SeqCst), 50);
    assert_eq!(r.gate.snapshot().active, 0);
    assert_eq!(r.gate.snapshot().acquired_total, 50);
}

#[tokio::test]
async fn a_busy_gate_surfaces_as_a_classifiable_error() {
    let r = rig(Script::Ok, false);
    let _held = r
        .gate
        .acquire(&MlxWorkerConfig::default())
        .await
        .expect("hold");
    let tight = GatedLocalModel::new(
        Arc::clone(&r.inner) as Arc<dyn ChatModel<()>>,
        "org/model",
        Arc::clone(&r.gate),
        Arc::clone(&r.metrics),
        Arc::clone(&r.worker) as Arc<dyn WorkerReady>,
        MlxWorkerConfig {
            max_waiters: 0,
            ..MlxWorkerConfig::default()
        },
    );
    let err = tight
        .invoke(&(), ModelRequest::default())
        .await
        .expect_err("busy");
    assert_eq!(gate_error_of(&err), Some(GateError::Busy));
}

#[test]
fn profile_and_cache_identity_pass_through() {
    let r = rig(Script::Ok, false);
    assert!(r.model.profile().is_none());
    assert_eq!(r.model.cache_identity(), r.inner.cache_identity());
}

// ── The factory's mlx branch ─────────────────────────────────────────────

fn factory(provider: &str, config: &Config) -> Arc<dyn ChatModel<()>> {
    crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
        provider, config,
    )
    .expect("local model builds")
    .0
}

#[test]
fn the_factory_gates_mlx_models_only_when_the_supervisor_is_on() {
    let mut config = Config::default();
    config.mlx.enabled = true;
    assert!(test_hook::is_gated(&factory("mlx:org/model", &config)));
    assert!(!test_hook::is_gated(&factory("ollama:llama3", &config)));
    assert!(!test_hook::is_gated(&factory(
        "lmstudio:some-model",
        &config
    )));

    config.mlx.enabled = false;
    assert!(!test_hook::is_gated(&factory("mlx:org/model", &config)));
}

// ---- priority ------------------------------------------------------------

fn model_on(gate: &Arc<InferenceGate>, inner: Arc<ScriptedModel>) -> Arc<GatedLocalModel> {
    Arc::new(GatedLocalModel::new(
        inner as Arc<dyn ChatModel<()>>,
        "m",
        Arc::clone(gate),
        Arc::new(MetricsSink::new()),
        Arc::new(FakeWorker {
            calls: AtomicUsize::new(0),
            fail: false,
            loading: false,
        }),
        MlxWorkerConfig {
            max_waiters: 64,
            acquire_timeout_secs: 30,
            ..MlxWorkerConfig::default()
        },
    ))
}

#[tokio::test]
async fn a_background_call_yields_to_chat_and_the_error_says_it_yielded() {
    use crate::neppy::inference::local::service::mlx_admin::gate::background_scope;

    let gate = Arc::new(InferenceGate::new().with_yield_after(Duration::from_millis(60)));
    let background = model_on(&gate, ScriptedModel::new(Script::Hang));
    let chat = model_on(&gate, ScriptedModel::new(Script::Ok));

    let task = {
        let background = Arc::clone(&background);
        tokio::spawn(background_scope(async move {
            background.invoke(&(), ModelRequest::default()).await
        }))
    };
    for _ in 0..200 {
        if gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        gate.snapshot().active,
        1,
        "the background call holds the slot"
    );

    // Chat waits behind a call that would never end by itself.
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        chat.invoke(&(), ModelRequest::default()),
    )
    .await
    .expect("chat was not starved by the background call")
    .expect("chat answers");
    assert_eq!(reply.usage.map(|u| u.output_tokens), Some(7));

    let err = task
        .await
        .expect("task")
        .expect_err("the background call was cancelled");
    assert_eq!(
        gate_error_of(&err),
        Some(GateError::Yielded),
        "a yield, not a memory preemption: {err}"
    );
    assert_eq!(gate.snapshot().active, 0);
}

#[tokio::test]
async fn a_call_outside_the_background_scope_is_never_asked_to_yield() {
    let gate = Arc::new(InferenceGate::new().with_yield_after(Duration::from_millis(40)));
    let holder = model_on(&gate, ScriptedModel::new(Script::Hang));
    let waiting = model_on(&gate, ScriptedModel::new(Script::Ok));
    let held = {
        let holder = Arc::clone(&holder);
        tokio::spawn(async move { holder.invoke(&(), ModelRequest::default()).await })
    };
    for _ in 0..200 {
        if gate.snapshot().active == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let second = {
        let waiting = Arc::clone(&waiting);
        tokio::spawn(async move { waiting.invoke(&(), ModelRequest::default()).await })
    };
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(!held.is_finished(), "an interactive holder keeps its slot");
    assert!(!second.is_finished());
    held.abort();
    second.abort();
}
