//! Deadline and empty-transcript behaviour of the Codex Responses client.
//!
//! Split out of `openai_codex_model_tests.rs` to keep that file near the
//! 500-line guideline. The servers here are self-contained because the shared
//! helper answers a whole response at once, and these tests need one that goes
//! quiet mid-body.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tinyagents::harness::message::Message;
use tinyagents::harness::model::{ModelRequest, ModelStreamItem};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::neppy::inference::provider::auth::AuthStyle;
use crate::neppy::inference::provider::crate_openai::{
    build_crate_openai_model, CrateOpenAiConfig,
};

fn config(endpoint: &str) -> CrateOpenAiConfig<'_> {
    CrateOpenAiConfig {
        provider_name: "openai",
        endpoint,
        api_key: "test-oauth-token",
        auth_style: AuthStyle::Bearer,
        model: "gpt-5.5",
        temperature_unsupported_models: &[],
        temperature_override: None,
        merge_system_into_user: false,
        extra_headers: &[],
        native_tool_calling: Some(true),
        vision: None,
        default_provider_options: None,
        responses_api_primary: true,
        responses_omit_max_output_tokens: true,
        extra_query_params: &[],
        user_agent: None,
    }
}

/// Answers 200 with the headers and one SSE delta, then holds the connection
/// open and silent: the headers arrive at once, so only a deadline on the BODY
/// can end the call before the 300s idle timeout. Counts accepted requests.
async fn spawn_stalling_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                // One read is enough: the reply does not depend on the body.
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let event = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"par\"}\n\n";
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{event}\r\n",
                    event.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            });
        }
    });
    (format!("http://{addr}"), hits)
}

/// `timeout_ms` used to bound only the wait for headers. A backend that
/// answers at once and then stalls mid-stream must now be cut off by it,
/// with a `timeout` error, well before the 300s idle timeout.
#[tokio::test]
async fn invoke_timeout_bounds_the_sse_body_not_just_the_headers() {
    let (base, _) = spawn_stalling_server().await;
    let model = build_crate_openai_model(config(&base));
    let started = Instant::now();

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        model.invoke(
            &(),
            ModelRequest::new(vec![Message::user("ping")]).with_timeout_ms(500),
        ),
    )
    .await
    .expect("the request deadline must end the call, not the 300s idle timeout");

    let err = outcome.unwrap_err();
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "took {:?}",
        started.elapsed()
    );
    match err {
        TinyAgentsError::Provider(p) => {
            assert_eq!(p.code.as_deref(), Some("timeout"), "error: {p:?}");
        }
        other => panic!("expected a provider timeout, got {other:?}"),
    }
}

/// Same deadline on the streaming path: the consumer sees a terminal failure
/// instead of a stream that never ends.
#[tokio::test]
async fn stream_timeout_ends_with_a_provider_failure_after_the_partial_delta() {
    let (base, _) = spawn_stalling_server().await;
    let model = build_crate_openai_model(config(&base));

    let stream = model
        .stream(
            &(),
            ModelRequest::new(vec![Message::user("ping")]).with_timeout_ms(500),
        )
        .await
        .expect("headers arrive before the deadline");
    let items: Vec<_> = tokio::time::timeout(Duration::from_secs(20), stream.collect())
        .await
        .expect("the deadline must close the stream");

    assert!(matches!(items.first(), Some(ModelStreamItem::Started)));
    match items.last() {
        Some(ModelStreamItem::ProviderFailed(p)) => {
            assert_eq!(p.code.as_deref(), Some("timeout"), "error: {p:?}");
        }
        other => panic!("expected ProviderFailed, got {other:?}"),
    }
}

/// A caller deadline does not cut off a reply that finishes inside it.
#[tokio::test]
async fn a_request_deadline_does_not_cut_off_a_reply_that_finishes_in_time() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let payload = concat!(
                    "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"ok\"}]}}\n\n",
                    "data: {\"type\":\"response.completed\",\"response\":{}}\n\n",
                );
                let reply = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    let model = build_crate_openai_model(config(&format!("http://{addr}")));

    let response = model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("ping")]).with_timeout_ms(10_000),
        )
        .await
        .expect("a reply that finishes inside the deadline succeeds");
    assert_eq!(response.text(), "ok");
}

/// Every user message is whitespace, so there is nothing to put in `input`.
/// The client refuses locally, non-retryably, and never contacts the backend.
#[tokio::test]
async fn whitespace_only_transcript_is_refused_locally_and_not_retryably() {
    let (base, hits) = spawn_stalling_server().await;
    let model = build_crate_openai_model(config(&base));

    let err = model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::system("be terse"), Message::user("  \n\t ")]),
        )
        .await
        .unwrap_err();

    match err {
        TinyAgentsError::Provider(p) => {
            assert_eq!(p.code.as_deref(), Some("invalid_request"));
            assert!(!p.retryable, "an empty transcript must not be retried");
            assert!(p.message.contains("no input"), "message: {}", p.message);
        }
        other => panic!("expected a provider error, got {other:?}"),
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0, "no request may be sent");
}

/// The stream entry point refuses the same way (it shares `post`).
#[tokio::test]
async fn whitespace_only_transcript_is_refused_on_the_stream_path_too() {
    let (base, hits) = spawn_stalling_server().await;
    let model = build_crate_openai_model(config(&base));

    let err = model
        .stream(&(), ModelRequest::new(vec![Message::user("   ")]))
        .await
        .err()
        .expect("an empty transcript cannot open a stream");

    assert!(matches!(err, TinyAgentsError::Provider(p) if !p.retryable));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}
