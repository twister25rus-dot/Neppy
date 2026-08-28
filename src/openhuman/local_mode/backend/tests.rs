//! End-to-end tests for the local backend.
//!
//! These drive a real listener over loopback rather than calling handlers
//! directly, because the contract under test is an HTTP one: the status codes,
//! the auth middleware and the fallback ordering are as much a part of it as
//! the JSON bodies, and none of them are exercised by a direct handler call.

use super::*;
use crate::openhuman::config::Config;

/// Serialize on the crate-wide backend env lock.
///
/// These tests read and clear the process-global published local-backend
/// address, which `local_mode::resolve`'s own tests also mutate. A
/// module-local mutex cannot prevent that cross-module race, and the symptom
/// would be an intermittent failure in whichever test lost — exactly the kind
/// of flake that gets re-run rather than fixed.
///
/// The guard is `!Send`, which is fine: `#[tokio::test]` polls the test future
/// on a current-thread runtime that never requires `Send`.
fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::api::config::backend_env_test_lock()
}

/// Boot a local backend on an ephemeral port.
fn test_config() -> Config {
    let mut config = Config::default();
    config.local_mode.enabled = true;
    // Ephemeral: the suite runs in parallel and a fixed port would make these
    // tests fail each other rather than fail themselves.
    config.local_mode.backend_port = 0;
    config
}

async fn boot() -> LocalBackendHandle {
    start(&test_config()).await.expect("local backend binds")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        // Never route loopback through a proxy the environment happens to set.
        .no_proxy()
        .build()
        .expect("client builds")
}

async fn get(handle: &LocalBackendHandle, path: &str) -> reqwest::Response {
    client()
        .get(format!("{}{path}", handle.base_url()))
        .bearer_auth(handle.session_token())
        .send()
        .await
        .expect("request completes")
}

#[tokio::test]
async fn binds_an_ephemeral_port_and_publishes_it() {
    let _serialized = test_lock();
    let handle = boot().await;
    assert_ne!(
        handle.addr().port(),
        0,
        "an ephemeral request must resolve to a real port"
    );
    assert!(
        handle.addr().ip().is_loopback(),
        "must never bind a routable interface"
    );
    assert_eq!(
        crate::openhuman::local_mode::local_backend_base_url(),
        handle.base_url(),
        "the bound address must be the one `effective_backend_api_url` resolves to"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn health_and_version_need_no_bearer() {
    let _serialized = test_lock();
    // A supervisor polls these before any token exists.
    let handle = boot().await;
    for path in ["/health", "/version"] {
        let response = client()
            .get(format!("{}{path}", handle.base_url()))
            .send()
            .await
            .expect("request completes");
        assert_eq!(response.status(), 200, "{path} must be public");
    }
    handle.shutdown().await;
}

#[tokio::test]
async fn every_other_route_requires_the_bearer() {
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .get(format!("{}/auth/me", handle.base_url()))
        .send()
        .await
        .expect("request completes");
    assert_eq!(
        response.status(),
        401,
        "a page the browser was induced to load must not be able to read the session"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn a_hosted_shaped_bearer_is_rejected() {
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .get(format!("{}/auth/me", handle.base_url()))
        .bearer_auth("header.payload.c2lnbmF0dXJl")
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 401);
    handle.shutdown().await;
}

#[tokio::test]
async fn the_renderers_own_local_token_is_accepted() {
    // The frontend mints its own `header.payload.local` on the "Continue
    // locally" path. Rejecting it would 401 the app's own UI on every call.
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .get(format!("{}/auth/me", handle.base_url()))
        .bearer_auth("eyJhbGciOiJub25lIn0.eyJzdWIiOiJsb2NhbCJ9.local")
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 200);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_website_origin_is_refused_even_with_a_valid_bearer() {
    // The attack the origin check exists for. CORS alone would not stop it:
    // a POST to the inference proxy does its damage on the way in, whether or
    // not the browser lets the page read the response.
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .post(format!("{}/openai/v1/chat/completions", handle.base_url()))
        .bearer_auth(handle.session_token())
        .header("Origin", "https://evil.example")
        .json(&serde_json::json!({ "model": "x", "messages": [] }))
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 403);
    handle.shutdown().await;
}

#[tokio::test]
async fn a_website_origin_is_refused_on_the_public_routes_too() {
    // `/health` skips the bearer, not the origin check.
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .get(format!("{}/health", handle.base_url()))
        .header("Origin", "https://evil.example")
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 403);
    handle.shutdown().await;
}

#[tokio::test]
async fn the_tauri_webview_origin_is_accepted() {
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .get(format!("{}/auth/me", handle.base_url()))
        .bearer_auth(handle.session_token())
        .header("Origin", "tauri://localhost")
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 200);
    handle.shutdown().await;
}

#[tokio::test]
async fn auth_me_returns_the_device_identity_in_the_hosted_shape() {
    let _serialized = test_lock();
    let handle = boot().await;
    let body: serde_json::Value = get(&handle, "/auth/me").await.json().await.expect("json");

    // `parse_api_response_value` unwraps `user` first, so this is the shape
    // `api::rest` hands to every caller.
    let user = body.get("user").expect("hosted payload nests the user");
    assert_eq!(user["id"], user["_id"], "both id spellings must agree");
    assert!(
        user["id"]
            .as_str()
            .expect("id is a string")
            .starts_with("local-"),
        "the device identity must be recognisable as local"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn refresh_returns_the_same_token_rather_than_rotating_it() {
    let _serialized = test_lock();
    // A rotation would invalidate the copy the renderer holds, and there is no
    // short-lived token to refresh in the first place.
    let handle = boot().await;
    let body: serde_json::Value = client()
        .post(format!("{}/auth/refresh", handle.base_url()))
        .bearer_auth(handle.session_token())
        .send()
        .await
        .expect("request completes")
        .json()
        .await
        .expect("json");
    assert_eq!(body["token"], handle.session_token());
    handle.shutdown().await;
}

#[tokio::test]
async fn team_usage_never_reads_as_an_exhausted_budget() {
    let _serialized = test_lock();
    // The invariant that keeps managed tools from being disabled forever on a
    // local install. Asserted through the real predicate, not by eyeballing
    // the payload.
    let handle = boot().await;
    let body: serde_json::Value = get(&handle, "/teams/me/usage")
        .await
        .json()
        .await
        .expect("json");
    let data = body.get("data").expect("hosted envelope carries data");

    assert!(
        !crate::openhuman::hosted::team::usage_budget_exhausted(data),
        "a local usage payload must never trip the managed budget gate"
    );
    assert_eq!(data["bypassCycleLimit"], true);
    assert_eq!(data["source"], "local");
    handle.shutdown().await;
}

#[tokio::test]
async fn announcements_are_absent_not_empty() {
    let _serialized = test_lock();
    let handle = boot().await;
    let response = get(&handle, "/announcements/latest").await;
    assert_eq!(
        response.status(),
        204,
        "an empty-object 200 would leave presence-checking clients rendering a blank banner"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn telemetry_is_accepted_and_dropped() {
    let _serialized = test_lock();
    // 202, not 501: a failing exporter makes real noise in exchange for telling
    // the user something they already chose.
    let handle = boot().await;
    let response = client()
        .post(format!(
            "{}/telemetry/langfuse/ingestion",
            handle.base_url()
        ))
        .bearer_auth(handle.session_token())
        .json(&serde_json::json!({ "batch": [{ "id": "1" }, { "id": "2" }] }))
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 202);
    handle.shutdown().await;
}

#[tokio::test]
async fn an_unsupported_hosted_route_answers_501_with_its_alternative() {
    let _serialized = test_lock();
    let handle = boot().await;
    let response = get(&handle, "/agent-integrations/composio/execute").await;
    assert_eq!(
        response.status(),
        501,
        "404 would say the route does not exist, which is false and invites a retry loop"
    );

    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["code"], super::error::LOCAL_MODE_UNSUPPORTED_CODE);
    assert_eq!(body["service_id"], "integrations.composio");
    assert!(
        body["local_alternative"]
            .as_str()
            .expect("alternative is a string")
            .contains("MCP"),
        "the error must name what to use instead"
    );
    handle.shutdown().await;
}

#[tokio::test]
async fn billing_says_it_is_absent_rather_than_reporting_a_zero_balance() {
    let _serialized = test_lock();
    // A `200 {"credits": 0}` would be a lie the UI cannot detect.
    let handle = boot().await;
    let response = get(&handle, "/payments/credits/balance").await;
    assert_eq!(response.status(), 501);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["service_id"], "account.billing");
    handle.shutdown().await;
}

#[tokio::test]
async fn the_fallback_covers_routes_no_service_entry_claims() {
    let _serialized = test_lock();
    // A hosted release that adds a route must still get an explained failure,
    // never a bare 404.
    let handle = boot().await;
    let response = get(&handle, "/some/future/hosted/route").await;
    assert_eq!(response.status(), 501);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["code"], super::error::LOCAL_MODE_UNSUPPORTED_CODE);
    assert!(body.get("service_id").is_none());
    handle.shutdown().await;
}

#[tokio::test]
async fn embeddings_reject_an_empty_input_before_touching_a_provider() {
    let _serialized = test_lock();
    let handle = boot().await;
    let response = client()
        .post(format!("{}/openai/v1/embeddings", handle.base_url()))
        .bearer_auth(handle.session_token())
        .json(&serde_json::json!({ "input": [] }))
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 400);
    handle.shutdown().await;
}

#[tokio::test]
async fn disabling_the_inference_proxy_makes_it_answer_unsupported() {
    let _serialized = test_lock();
    let mut config = test_config();
    config.local_mode.proxy_inference = false;
    let handle = start(&config).await.expect("binds");

    let response = client()
        .post(format!("{}/openai/v1/chat/completions", handle.base_url()))
        .bearer_auth(handle.session_token())
        .json(&serde_json::json!({ "model": "x", "messages": [] }))
        .send()
        .await
        .expect("request completes");
    assert_eq!(response.status(), 501);
    handle.shutdown().await;
}

#[tokio::test]
async fn shutdown_clears_the_published_address() {
    let _serialized = test_lock();
    let handle = boot().await;
    let base_url = handle.base_url().to_string();
    assert_eq!(
        crate::openhuman::local_mode::local_backend_base_url(),
        base_url
    );

    handle.shutdown().await;

    assert_ne!(
        crate::openhuman::local_mode::local_backend_base_url(),
        base_url,
        "a stopped server must not keep advertising an address nothing is listening on"
    );
}
