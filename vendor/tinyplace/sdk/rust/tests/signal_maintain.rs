//! Coverage for idempotent key-bundle maintenance
//! ([`tinyplace::signal::maintain::maintain_keys`]).
//!
//! The signed pre-key and the one-time pool are maintained **independently**, so
//! each test pins one half healthy and the other degraded to prove neither
//! drags the other into a needless republish.

mod common;

use common::{all_requests, client_for, test_signer};
use serde_json::{json, Value};
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

use tinyplace::signal::crypto::generate_x25519_keypair;
use tinyplace::signal::keys::generate_signed_pre_key;
use tinyplace::signal::maintain::{maintain_keys, MaintainPolicy};
use tinyplace::signal::memory_store::MemorySessionStore;
use tinyplace::signal::store::SessionStore;
use tinyplace::{LocalSigner, Signer};

/// A relay mock whose `GET /health` reports `one_time_count` one-time pre-keys
/// and advertises `signed_id` as its signed pre-key, and whose `PUT`s (rotate /
/// upload) return `null`.
async fn relay(one_time_count: i64, signed_id: Value) -> MockServer {
    let server = MockServer::start().await;
    // Pinned to the real health route (the id segment is the agent's base58 key),
    // so a wrong path would fail the scenario instead of silently matching.
    Mock::given(method("GET"))
        .and(path_regex(r"^/keys/[^/]+/health$"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "agentId": "x",
            "oneTimePreKeyCount": one_time_count,
            "lowOneTimePreKeys": one_time_count < 5,
            "signedPreKeyKeyId": signed_id,
        })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!(null)))
        .mount(&server)
        .await;
    server
}

/// A store already backing a signed pre-key under `key_id`.
async fn store_backing(signer: &LocalSigner, key_id: &str) -> MemorySessionStore {
    let store = MemorySessionStore::new(generate_x25519_keypair());
    let spk = generate_signed_pre_key(signer, key_id).await.unwrap();
    store.store_signed_pre_key(spk).await.unwrap();
    store
}

/// Count the `PUT`s that hit each key route.
async fn puts(server: &MockServer) -> (usize, usize) {
    let requests = all_requests(server).await;
    let count = |suffix: &str| {
        requests
            .iter()
            .filter(|r| r.method.as_str() == "PUT" && r.url.path().ends_with(suffix))
            .count()
    };
    (count("/signed-prekey"), count("/prekeys"))
}

#[tokio::test]
async fn no_ops_when_the_advertised_signed_key_is_backed_and_the_pool_is_healthy() {
    let signer = test_signer();
    let server = relay(20, json!("spk_known")).await;
    let client = client_for(&server);
    let store = store_backing(&signer, "spk_known").await;

    let report = maintain_keys(
        &client.keys,
        &store,
        signer.as_ref(),
        &signer.agent_id(),
        "identity-key",
        &MaintainPolicy::default(),
    )
    .await
    .unwrap();

    assert!(report.was_healthy);
    assert!(!report.rotated_signed);
    assert_eq!(report.uploaded_one_time, 0);
    assert_eq!(
        puts(&server).await,
        (0, 0),
        "a healthy agent publishes nothing"
    );
}

#[tokio::test]
async fn rotates_only_the_signed_key_when_the_relay_advertises_one_we_cannot_back() {
    // The relay serves `spk_missing`, which the store does not hold — an inbound
    // PREKEY_BUNDLE naming it would fail to decrypt, so maintenance must repair
    // it. The one-time pool is healthy and must be left alone.
    let signer = test_signer();
    let server = relay(20, json!("spk_missing")).await;
    let client = client_for(&server);
    let store = store_backing(&signer, "spk_other").await;

    let report = maintain_keys(
        &client.keys,
        &store,
        signer.as_ref(),
        &signer.agent_id(),
        "identity-key",
        &MaintainPolicy::default(),
    )
    .await
    .unwrap();

    assert!(!report.was_healthy);
    assert!(report.rotated_signed);
    assert_eq!(
        report.uploaded_one_time, 0,
        "a healthy pool is not disturbed"
    );
    assert_eq!(puts(&server).await, (1, 0));
}

#[tokio::test]
async fn tops_up_only_the_pool_when_low_leaving_a_good_signed_key_alone() {
    // Peers have drawn the pool DOWN but not to zero: the relay still reports a
    // positive count and only raises `lowOneTimePreKeys`. This pins the low-flag
    // branch specifically (the empty-pool `count <= 0` branch is covered by the
    // first-boot case), while the advertised signed pre-key is still backed — so
    // refill must happen without any signed-key churn.
    let signer = test_signer();
    let server = relay(4, json!("spk_known")).await;
    let client = client_for(&server);
    let store = store_backing(&signer, "spk_known").await;

    let report = maintain_keys(
        &client.keys,
        &store,
        signer.as_ref(),
        &signer.agent_id(),
        "identity-key",
        &MaintainPolicy::default(),
    )
    .await
    .unwrap();

    assert!(!report.was_healthy);
    assert!(!report.rotated_signed, "signed key must not be rotated");
    assert_eq!(report.uploaded_one_time, 20);
    assert_eq!(puts(&server).await, (0, 1));
}

#[tokio::test]
async fn publishes_both_halves_on_first_boot() {
    // Nothing published yet: the relay advertises no signed pre-key and an empty
    // pool, and the store is empty.
    let signer = test_signer();
    let server = relay(0, json!(null)).await;
    let client = client_for(&server);
    let store = MemorySessionStore::new(generate_x25519_keypair());

    let report = maintain_keys(
        &client.keys,
        &store,
        signer.as_ref(),
        &signer.agent_id(),
        "identity-key",
        &MaintainPolicy::default(),
    )
    .await
    .unwrap();

    assert!(!report.was_healthy);
    assert!(report.rotated_signed);
    assert_eq!(report.uploaded_one_time, 20);
    assert_eq!(puts(&server).await, (1, 1));
}
