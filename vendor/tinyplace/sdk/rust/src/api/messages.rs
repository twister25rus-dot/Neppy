//! Direct (1:1) message relay. Mirrors `sdk/typescript/src/api/messages.ts`.
//!
//! This is a plain REST wrapper: it carries whatever `body` ciphertext/string
//! the caller supplies and performs no Signal/E2E encryption.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::MessageEnvelope;
use crate::util::encode;

/// The `{ messages: [...] }` response wrapper from `GET /messages`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageListResponse {
    pub messages: Vec<MessageEnvelope>,
}

#[derive(Clone)]
pub struct MessagesApi {
    http: HttpClient,
}

impl MessagesApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    /// List messages addressed to `agent_id`.
    pub async fn list(&self, agent_id: &str, limit: Option<i64>) -> Result<MessageListResponse> {
        let mut query: Vec<(String, String)> = vec![("agentId".into(), agent_id.to_string())];
        if let Some(limit) = limit {
            query.push(("limit".into(), limit.to_string()));
        }
        self.http
            .get_directory_auth_as("/messages", agent_id, &query)
            .await
    }

    /// Send a message envelope (carries an opaque `body`).
    pub async fn send(&self, mut envelope: MessageEnvelope) -> Result<MessageEnvelope> {
        if envelope.timestamp.is_empty() {
            envelope.timestamp = crate::auth::timestamp();
        }
        let actor = envelope.from.clone();
        self.http
            .put_directory_auth_as("/messages", &actor, Some(&envelope))
            .await
    }

    /// Acknowledge (delete) a delivered message.
    pub async fn acknowledge(&self, message_id: &str, agent_id: &str) -> Result<()> {
        let path = format!(
            "/messages/{}?agentId={}",
            encode(message_id),
            encode(agent_id)
        );
        self.http
            .delete_directory_auth_as::<(), serde_json::Value>(&path, agent_id, None)
            .await
    }
}
