//! Idempotent key-bundle maintenance.
//!
//! Reconciles an agent's *published* pre-key bundle (on the relay) with the
//! private material in its [`SessionStore`], generating and uploading keys ONLY
//! when the relay actually needs them.
//!
//! This is the anti-orphan invariant. Blindly republishing a fresh bundle on
//! every start leaves the relay advertising one-time pre-keys whose private
//! halves a rotated or wiped store can no longer back, so a peer's X3DH against
//! that bundle fails with a MAC error and its first message is silently dropped.
//! Storing the private material locally *before* uploading, and skipping the
//! upload entirely when the store already backs a healthy published set,
//! guarantees the relay never serves a key this agent cannot answer.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::keys::KeysApi;
use crate::error::Result;
use crate::signal::keys::{generate_pre_keys, generate_signed_pre_key, serialize_pre_key};
use crate::signal::store::SessionStore;
use crate::signer::Signer;
use crate::types::{PreKeysRequest, SignedPreKeyRequest};

/// Tuning for [`maintain_keys`].
#[derive(Debug, Clone)]
pub struct MaintainPolicy {
    /// How many one-time pre-keys to (re)publish in a batch when the pool needs
    /// filling.
    pub one_time_batch: usize,
}

impl Default for MaintainPolicy {
    fn default() -> Self {
        Self { one_time_batch: 20 }
    }
}

/// What [`maintain_keys`] did on a call.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyMaintenance {
    /// Nothing needed publishing: the store backed the signed pre-key the relay
    /// advertises and its one-time pool was healthy — the common restart path.
    pub was_healthy: bool,
    /// A fresh signed pre-key was generated and rotated on the relay, because the
    /// store could not answer the id the relay was advertising.
    pub rotated_signed: bool,
    /// How many one-time pre-keys were generated and uploaded.
    pub uploaded_one_time: usize,
}

/// Idempotently reconcile `agent_id`'s published key bundle with the relay.
///
/// Safe to call on every boot and periodically — it publishes only what the
/// relay actually needs. The two halves are maintained **independently**:
///
/// - **Signed pre-key** — rotated only when the store cannot answer the id the
///   relay advertises (`KeyHealth::signed_pre_key_key_id`). An inbound
///   `PREKEY_BUNDLE` names that exact id, so a relay serving a key we no longer
///   hold breaks first contact; a still-backed signed key is left untouched.
/// - **One-time pre-keys** — topped up only when the relay reports the pool low
///   or empty, without disturbing a healthy signed pre-key.
///
/// So a merely-depleted one-time pool refills without needless signed-key
/// churn, and a stale/unbacked signed key is repaired even while the pool is
/// full. When both are satisfied the call is a **no-op**
/// ([`KeyMaintenance::was_healthy`]) — not republishing is what prevents
/// orphaning. If the health check itself fails, both halves are (re)published
/// rather than risk leaving the agent unpublished.
///
/// `identity_key_base64` is the agent's base64 Ed25519 identity key; the relay
/// verifies each published key's signature against it.
///
/// # Errors
///
/// Returns the underlying error if key generation, a store write, the signed
/// pre-key rotation, or the one-time upload fails. A failed *health* check is
/// treated as "cannot confirm healthy" and falls through to a (re)publish rather
/// than erroring, so a transient relay blip never leaves the agent unpublished.
pub async fn maintain_keys(
    keys: &KeysApi,
    store: &dyn SessionStore,
    signer: &dyn Signer,
    agent_id: &str,
    identity_key_base64: &str,
    policy: &MaintainPolicy,
) -> Result<KeyMaintenance> {
    // What is the relay actually serving right now? If we cannot reach it we
    // cannot confirm anything is healthy, so both halves are (re)published
    // rather than risk leaving the agent unpublished.
    let health = keys.health(agent_id).await.ok();

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut report = KeyMaintenance::default();

    // --- Signed pre-key -----------------------------------------------------
    // Rotate ONLY when the store cannot answer the signed pre-key the relay
    // advertises. An inbound `PREKEY_BUNDLE` names that exact id, so a relay
    // serving an id we no longer hold makes first contact fail; conversely a
    // still-backed signed key is left alone even when the one-time pool needs a
    // refill (the two halves are maintained independently).
    let signed_pre_key_backed = match health
        .as_ref()
        .and_then(|h| h.signed_pre_key_key_id.as_deref())
    {
        Some(key_id) => store.signed_pre_key(key_id).await.ok().flatten().is_some(),
        // No health, or the relay advertises no signed pre-key at all.
        None => false,
    };
    if !signed_pre_key_backed {
        let spk = generate_signed_pre_key(signer, &format!("spk_{now_secs}")).await?;
        store.store_signed_pre_key(spk.clone()).await?;
        keys.rotate_signed_pre_key(
            agent_id,
            &SignedPreKeyRequest {
                identity_key: Some(identity_key_base64.to_string()),
                signed_pre_key: serialize_pre_key(&spk),
            },
        )
        .await?;
        report.rotated_signed = true;
    }

    // --- One-time pre-keys --------------------------------------------------
    // Topped up independently, driven only by the relay's own pool report. The
    // private halves are stored BEFORE upload so the relay never advertises a
    // key the store cannot back.
    let one_time_depleted = match health.as_ref() {
        Some(h) => h.low_one_time_pre_keys || h.one_time_pre_key_count <= 0,
        None => true,
    };
    if one_time_depleted {
        let one_time = generate_pre_keys(signer, now_secs, policy.one_time_batch).await?;
        for key in &one_time {
            store.store_pre_key(key.clone()).await?;
        }
        keys.upload_pre_keys(
            agent_id,
            &PreKeysRequest {
                identity_key: Some(identity_key_base64.to_string()),
                pre_keys: one_time.iter().map(serialize_pre_key).collect(),
            },
        )
        .await?;
        report.uploaded_one_time = one_time.len();
    }

    report.was_healthy = !report.rotated_signed && report.uploaded_one_time == 0;
    Ok(report)
}
