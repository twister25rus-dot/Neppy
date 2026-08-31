//! WebSocket streaming tests: signed-URL construction, the signed upgrade
//! headers the backend authenticates the handshake with (public API), and a live
//! connect/recv/send/close round-trip against a local `tokio-tungstenite` server.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::{SinkExt as _, StreamExt as _};
use tinyplace::{LocalSigner, Signer as _, TinyPlaceClient, TinyPlaceClientOptions};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;

fn client(base_url: &str, with_signer: bool) -> TinyPlaceClient {
    let signer = with_signer.then(|| {
        Arc::new(LocalSigner::from_seed(&[1u8; 32]).unwrap()) as Arc<dyn tinyplace::Signer>
    });
    TinyPlaceClient::new(TinyPlaceClientOptions {
        base_url: base_url.to_string(),
        signer,
        ..Default::default()
    })
}

/// A client whose signer opts out of SIWS, so authenticated requests carry the
/// per-request freshness-bound `v1:` token instead of a reusable proof.
fn client_without_siws(base_url: &str) -> (TinyPlaceClient, LocalSigner) {
    let signer = LocalSigner::from_seed(&[3u8; 32]).unwrap().without_siws();
    let client = TinyPlaceClient::new(TinyPlaceClientOptions {
        base_url: base_url.to_string(),
        signer: Some(Arc::new(signer.clone()) as Arc<dyn tinyplace::Signer>),
        ..Default::default()
    });
    (client, signer)
}

fn header_map(headers: Vec<(String, String)>) -> HashMap<String, String> {
    headers
        .into_iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect()
}

#[tokio::test]
async fn ws_url_maps_https_to_wss() {
    let client = client("https://api.example.com", false);
    let url = client.inbox.stream().signed_url().await.unwrap();
    assert_eq!(url, "wss://api.example.com/inbox/stream");
}

#[tokio::test]
async fn ws_url_maps_http_to_ws() {
    let client = client("http://localhost:8080", false);
    let url = client.inbox.stream().signed_url().await.unwrap();
    assert_eq!(url, "ws://localhost:8080/inbox/stream");
}

#[tokio::test]
async fn ws_agent_auth_appends_authorization_param() {
    let client = client("https://api.example.com", true);
    let url = client.activity.stream(None).signed_url().await.unwrap();
    assert!(url.contains("authorization="), "got: {url}");
    assert!(!url.contains("X-TinyPlace-Signature="), "got: {url}");
}

#[tokio::test]
async fn ws_directory_auth_signs_query_params() {
    let client = client("https://api.example.com", true);
    let url = client.a2a.stream("@alice").signed_url().await.unwrap();
    assert!(url.contains("X-TinyPlace-Public-Key="), "got: {url}");
    assert!(url.contains("X-TinyPlace-Signature="), "got: {url}");
    assert!(url.contains("X-TinyPlace-Date="), "got: {url}");
    assert!(url.contains("X-TinyPlace-Nonce="), "got: {url}");
}

#[tokio::test]
async fn ws_no_signer_no_auth_params() {
    let client = client("https://api.example.com", false);
    let url = client.activity.stream(None).signed_url().await.unwrap();
    assert_eq!(url, "wss://api.example.com/activity/stream");
}

#[tokio::test]
async fn ws_stream_query_params_are_included() {
    let client = client("https://api.example.com", false);
    let url = client
        .channels
        .stream("c1", None, Some(50))
        .signed_url()
        .await
        .unwrap();
    assert!(url.starts_with("wss://api.example.com/channels/c1/stream?"));
    assert!(url.contains("limit=50"), "got: {url}");
}

#[tokio::test]
async fn ws_upgrade_headers_carry_the_signed_credential() {
    let client = client("https://api.example.com", true);
    let headers = header_map(
        client
            .inbox
            .stream()
            .upgrade_headers()
            .await
            .expect("upgrade headers"),
    );

    // The backend authenticates the handshake on X-TinyPlace-Signature, keyed to
    // the public key presented alongside it.
    let signature = headers
        .get("x-tinyplace-signature")
        .expect("signature header");
    assert!(signature.starts_with("siws:"), "got: {signature}");
    assert_eq!(
        headers.get("x-tinyplace-public-key").map(String::as_str),
        Some(
            LocalSigner::from_seed(&[1u8; 32])
                .unwrap()
                .public_key_base64()
        )
        .as_deref()
    );
    assert_eq!(
        headers.get("x-tinyplace-crypto-id").map(String::as_str),
        Some(LocalSigner::from_seed(&[1u8; 32]).unwrap().agent_id()).as_deref()
    );
    assert_eq!(
        headers.get("x-tinyplace-sdk").map(String::as_str),
        Some(tinyplace::SDK_CLIENT)
    );
}

#[tokio::test]
async fn ws_upgrade_signature_signs_the_empty_canonical_payload() {
    let (client, signer) = client_without_siws("https://api.example.com");
    let headers = header_map(
        client
            .a2a
            .stream("@alice")
            .upgrade_headers()
            .await
            .expect("upgrade headers"),
    );

    // v1:<b64url(timestamp)>:<b64url(nonce)>:<base64(signature)>, signed over
    // `<canonical payload>\n<timestamp>\n<nonce>`. A stream GET declares no
    // action and no fields, so the payload is the empty canonical envelope.
    let token = headers
        .get("x-tinyplace-signature")
        .expect("signature header");
    let parts: Vec<&str> = token.split(':').collect();
    assert_eq!(parts.len(), 4, "got: {token}");
    assert_eq!(parts[0], "v1");

    let url_safe = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let timestamp = String::from_utf8(url_safe.decode(parts[1]).unwrap()).unwrap();
    let nonce = String::from_utf8(url_safe.decode(parts[2]).unwrap()).unwrap();
    let signature = base64::engine::general_purpose::STANDARD
        .decode(parts[3])
        .unwrap();
    assert!(
        timestamp.contains('T') && timestamp.ends_with('Z'),
        "{timestamp}"
    );
    assert!(!nonce.is_empty());

    let payload = format!("{{\"action\":\"\",\"fields\":{{}}}}\n{timestamp}\n{nonce}");
    let verifying = VerifyingKey::from_bytes(signer.public_key()).unwrap();
    verifying
        .verify(
            payload.as_bytes(),
            &Signature::from_slice(&signature).unwrap(),
        )
        .expect("upgrade signature verifies against the signer's key");
}

#[tokio::test]
async fn ws_upgrade_headers_without_a_signer_carry_only_the_sdk_tag() {
    let client = client("https://api.example.com", false);
    let headers = header_map(
        client
            .activity
            .stream(None)
            .upgrade_headers()
            .await
            .expect("upgrade headers"),
    );
    assert_eq!(headers.len(), 1);
    assert!(headers.contains_key("x-tinyplace-sdk"));
}

// The handshake callback's error type is tungstenite's own `ErrorResponse`.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn ws_handshake_sends_the_signed_headers_to_the_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel::<HashMap<String, String>>();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws =
            tokio_tungstenite::accept_hdr_async(tcp, |request: &Request, response: Response| {
                let seen = request
                    .headers()
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_lowercase(),
                            value.to_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                sender.send(seen).unwrap();
                Ok(response)
            })
            .await
            .unwrap();
        let _ = ws.close(None).await;
    });

    let client = client(&format!("http://{addr}"), true);
    let conn = client.inbox.stream().connect().await.unwrap();
    conn.close().await.unwrap();
    server.await.unwrap();

    let seen = receiver.recv().unwrap();
    assert!(
        seen.get("x-tinyplace-signature")
            .is_some_and(|value| value.starts_with("siws:")),
        "handshake reached the server without a signature header: {seen:?}"
    );
    assert!(seen.contains_key("x-tinyplace-public-key"));
    assert!(seen.contains_key("x-tinyplace-crypto-id"));
    assert_eq!(
        seen.get("x-tinyplace-sdk").map(String::as_str),
        Some(tinyplace::SDK_CLIENT)
    );
}

#[tokio::test]
async fn ws_connect_recv_send_close_round_trip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ws.send(Message::Text("{\"type\":\"hello\",\"n\":1}".to_string()))
            .await
            .unwrap();
        // Echo back whatever the client sends, then close.
        if let Some(Ok(incoming)) = ws.next().await {
            if incoming.is_text() {
                ws.send(incoming).await.unwrap();
            }
        }
        let _ = ws.close(None).await;
    });

    let client = client(&format!("http://{addr}"), false);
    let mut conn = client.inbox.stream().connect().await.unwrap();

    let first = conn.recv().await.expect("a message").expect("valid json");
    assert_eq!(first["type"], "hello");
    assert_eq!(first["n"], 1);

    conn.send(&serde_json::json!({"ack": true})).await.unwrap();
    let echoed = conn.recv().await.expect("echo").expect("valid json");
    assert_eq!(echoed["ack"], true);

    conn.close().await.unwrap();
    server.await.unwrap();
}

/// A local server that serves `rounds` successive connections, sending one
/// frame on each before closing it. Returns the bound address and the task; the
/// listener is dropped once every round has been served, so a later dial is
/// refused rather than hanging.
async fn flapping_server(rounds: usize) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        for n in 1..=rounds {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text(format!("{{\"type\":\"tick\",\"n\":{n}}}")))
                .await
                .unwrap();
            let _ = ws.close(None).await;
        }
        drop(listener);
    });
    (format!("http://{addr}"), task)
}

fn fast_reconnect(max_attempts: u32) -> tinyplace::ReconnectPolicy {
    tinyplace::ReconnectPolicy {
        enabled: true,
        interval: Duration::from_millis(5),
        max_attempts,
        connect_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn ws_reconnect_defaults_match_the_typescript_sdk() {
    // Parity with `TinyPlaceWebSocketOptions` in sdk/typescript/src/websocket.ts.
    let policy = tinyplace::ReconnectPolicy::default();
    assert!(policy.enabled);
    assert_eq!(policy.interval, Duration::from_millis(3_000));
    assert_eq!(policy.max_attempts, 10);
    assert!(!tinyplace::ReconnectPolicy::disabled().enabled);
}

#[tokio::test]
async fn ws_keepalive_traffic_resets_the_retry_budget() {
    // A quiet stream — an inbox with no new items — carries nothing but the
    // server's keepalive pings. If only data frames reset the budget, outages
    // separated by healthy-but-silent connections accumulate until
    // `max_attempts` is spent and the stream ends for good.
    //
    // With `max_attempts: 1`, surviving to the third connection is only
    // possible if the pings on connections one and two reset the budget.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for round in 1..=3 {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            if round == 3 {
                ws.send(Message::Text("{\"n\":3}".to_string()))
                    .await
                    .unwrap();
            } else {
                ws.send(Message::Ping(Vec::new())).await.unwrap();
            }
            let _ = ws.close(None).await;
        }
    });

    let client = client(&format!("http://{addr}"), false);
    let mut conn = client
        .inbox
        .stream()
        .with_reconnect(fast_reconnect(1))
        .connect()
        .await
        .unwrap();

    let frame = tokio::time::timeout(Duration::from_secs(5), conn.recv())
        .await
        .expect("recv must not hang")
        .expect("the keepalives should have kept the budget alive")
        .expect("valid json");
    assert_eq!(frame["n"], 3);

    server.await.unwrap();
}

#[tokio::test]
async fn ws_reconnect_bounds_a_stalled_handshake() {
    // `connect_async` has no timeout of its own, so a server that accepts the
    // TCP connection but never completes the upgrade would hang `recv` forever
    // and leave `max_attempts` bounding nothing.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ws.send(Message::Text("{\"n\":1}".to_string()))
            .await
            .unwrap();
        let _ = ws.close(None).await;
        // Accept, then stall: never answer the upgrade, and hold the socket
        // open so the dial cannot fail fast on a closed connection either.
        let mut stalled = Vec::new();
        loop {
            if let Ok((tcp, _)) = listener.accept().await {
                stalled.push(tcp);
            }
        }
    });

    let client = client(&format!("http://{addr}"), false);
    let mut conn = client
        .inbox
        .stream()
        .with_reconnect(tinyplace::ReconnectPolicy {
            enabled: true,
            interval: Duration::from_millis(5),
            max_attempts: 2,
            connect_timeout: Duration::from_millis(50),
        })
        .connect()
        .await
        .unwrap();

    assert_eq!(
        conn.recv().await.expect("first frame").expect("valid json")["n"],
        1
    );

    // Two attempts, each capped at 50ms — comfortably inside this bound. Without
    // the per-attempt timeout this hangs until the test harness gives up.
    let ended = tokio::time::timeout(Duration::from_secs(5), conn.recv())
        .await
        .expect("a stalled handshake must not hang recv");
    assert!(ended.is_none(), "expected the stream to end, got {ended:?}");

    server.abort();
}

#[tokio::test]
async fn ws_recv_reconnects_after_the_server_drops_the_stream() {
    let (url, server) = flapping_server(2).await;
    let client = client(&url, false);
    let mut conn = client
        .inbox
        .stream()
        .with_reconnect(fast_reconnect(5))
        .connect()
        .await
        .unwrap();

    // The first frame arrives on the original socket; the server then closes.
    let first = conn.recv().await.expect("first frame").expect("valid json");
    assert_eq!(first["n"], 1);

    // The drop is invisible to the caller: recv re-dials and yields the frame
    // the *second* connection carries.
    let second = conn
        .recv()
        .await
        .expect("second frame")
        .expect("valid json");
    assert_eq!(second["n"], 2);

    server.await.unwrap();
}

#[tokio::test]
async fn ws_recv_reports_the_drop_when_reconnecting_is_disabled() {
    let (url, server) = flapping_server(1).await;
    let client = client(&url, false);
    let mut conn = client
        .inbox
        .stream()
        .with_reconnect(tinyplace::ReconnectPolicy::disabled())
        .connect()
        .await
        .unwrap();

    let first = conn.recv().await.expect("first frame").expect("valid json");
    assert_eq!(first["n"], 1);
    // Previous behaviour, preserved as an opt-out: the close ends the stream.
    assert!(conn.recv().await.is_none(), "expected the stream to end");

    server.await.unwrap();
}

#[tokio::test]
async fn ws_recv_gives_up_after_max_attempts() {
    // One round only: after it, the listener is gone and every re-dial fails.
    let (url, server) = flapping_server(1).await;
    let client = client(&url, false);
    let mut conn = client
        .inbox
        .stream()
        .with_reconnect(fast_reconnect(3))
        .connect()
        .await
        .unwrap();

    assert_eq!(
        conn.recv().await.expect("first frame").expect("valid json")["n"],
        1
    );

    // Bounded, not infinite: three failed attempts and it reports the end.
    let ended = tokio::time::timeout(Duration::from_secs(5), conn.recv())
        .await
        .expect("recv must not hang once attempts are exhausted");
    assert!(ended.is_none(), "expected the stream to end, got {ended:?}");

    server.await.unwrap();
}

// The handshake callback's error type is tungstenite's own `ErrorResponse`.
#[allow(clippy::result_large_err)]
#[tokio::test]
async fn ws_reconnect_signs_a_fresh_upgrade() {
    // The upgrade credential is freshness-bound (`v1:<ts>:<nonce>:<sig>`), so a
    // reconnect must re-sign rather than replay the original request — a
    // replayed nonce is exactly what the backend rejects with a 401.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel::<HashMap<String, String>>();

    let server = tokio::spawn(async move {
        for n in 1..=2 {
            let (tcp, _) = listener.accept().await.unwrap();
            let sender = sender.clone();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                tcp,
                |request: &Request, response: Response| {
                    let seen = request
                        .headers()
                        .iter()
                        .map(|(name, value)| {
                            (
                                name.as_str().to_lowercase(),
                                value.to_str().unwrap_or_default().to_string(),
                            )
                        })
                        .collect();
                    sender.send(seen).unwrap();
                    Ok(response)
                },
            )
            .await
            .unwrap();
            ws.send(Message::Text(format!("{{\"n\":{n}}}")))
                .await
                .unwrap();
            let _ = ws.close(None).await;
        }
    });

    let (client, _signer) = client_without_siws(&format!("http://{addr}"));
    let mut conn = client
        .inbox
        .stream()
        .with_reconnect(fast_reconnect(5))
        .connect()
        .await
        .unwrap();
    assert_eq!(conn.recv().await.unwrap().unwrap()["n"], 1);
    assert_eq!(conn.recv().await.unwrap().unwrap()["n"], 2);
    server.await.unwrap();

    let first = receiver.recv().unwrap();
    let second = receiver.recv().unwrap();
    let signature = |headers: &HashMap<String, String>| {
        headers
            .get("x-tinyplace-signature")
            .cloned()
            .expect("signature header")
    };
    let (first, second) = (signature(&first), signature(&second));
    assert!(first.starts_with("v1:"), "got: {first}");
    assert!(second.starts_with("v1:"), "got: {second}");
    assert_ne!(
        first, second,
        "the reconnect replayed the original credential instead of re-signing"
    );
}
