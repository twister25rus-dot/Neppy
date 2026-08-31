//! Signal pre-key bundle management. Mirrors `sdk/typescript/src/api/keys.ts`.
//!
//! Plain REST: this fetches/uploads the opaque key material the backend stores;
//! it does not generate or validate any Signal keys itself.

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::{KeyBundle, KeyHealth, PreKeysRequest, SignedPreKeyRequest};
use crate::util::encode;

#[derive(Clone)]
pub struct KeysApi {
    http: HttpClient,
}

impl KeysApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    /// Fetch an agent's published pre-key bundle (public).
    pub async fn get_bundle(&self, agent_id: &str) -> Result<KeyBundle> {
        let path = format!("/keys/{}/bundle", encode(agent_id));
        self.http.get(&path, &[]).await
    }

    /// Inspect the health of an agent's one-time pre-key supply.
    pub async fn health(&self, agent_id: &str) -> Result<KeyHealth> {
        let path = format!("/keys/{}/health", encode(agent_id));
        self.http.get_directory_auth_as(&path, agent_id, &[]).await
    }

    /// Upload a batch of one-time pre-keys.
    pub async fn upload_pre_keys(&self, agent_id: &str, request: &PreKeysRequest) -> Result<()> {
        let path = format!("/keys/{}/prekeys", encode(agent_id));
        self.http
            .put_directory_auth_as(&path, agent_id, Some(request))
            .await
    }

    /// Rotate the agent's signed pre-key.
    pub async fn rotate_signed_pre_key(
        &self,
        agent_id: &str,
        request: &SignedPreKeyRequest,
    ) -> Result<()> {
        let path = format!("/keys/{}/signed-prekey", encode(agent_id));
        self.http
            .put_directory_auth_as(&path, agent_id, Some(request))
            .await
    }
}
