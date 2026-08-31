//! Administrative operations (`/admin`). All endpoints use admin auth.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::{
    AdminAuditEntry, AdminFeeMetrics, AgentPaymentStatus, FeeConfig, FeeResolveParams,
    FeeResolveResponse, SystemConfig,
};
use crate::util::encode;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeesResponse {
    pub fees: Vec<FeeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResponse {
    pub config: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditResponse {
    pub audit: Vec<AdminAuditEntry>,
}

/// Body for [`AdminApi::suspend_agent`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuspendAgentParams {
    pub until: String,
    pub reason: String,
}

/// Filters for [`AdminApi::audit`].
#[derive(Debug, Clone, Default)]
pub struct AuditQueryParams {
    pub actor: Option<String>,
    pub action: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Clone)]
pub struct AdminApi {
    http: HttpClient,
}

impl AdminApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    // --- Fee Configuration ---

    pub async fn list_fees(&self) -> Result<FeesResponse> {
        self.http.get_admin("/admin/fees", &[]).await
    }

    pub async fn create_fee(&self, fee: &FeeConfig) -> Result<FeeConfig> {
        self.http.post_admin("/admin/fees", Some(fee)).await
    }

    pub async fn get_fee(&self, fee_id: &str) -> Result<FeeConfig> {
        let path = format!("/admin/fees/{}", encode(fee_id));
        self.http.get_admin(&path, &[]).await
    }

    pub async fn update_fee(&self, fee_id: &str, update: &FeeConfig) -> Result<FeeConfig> {
        let path = format!("/admin/fees/{}", encode(fee_id));
        self.http.put_admin(&path, Some(update)).await
    }

    pub async fn delete_fee(&self, fee_id: &str) -> Result<()> {
        let path = format!("/admin/fees/{}", encode(fee_id));
        self.http
            .delete_admin::<(), serde_json::Value>(&path, None)
            .await
    }

    pub async fn resolve_fee(&self, params: &FeeResolveParams) -> Result<FeeResolveResponse> {
        let mut q: Vec<(String, String)> = vec![
            ("from".into(), params.from.clone()),
            ("to".into(), params.to.clone()),
        ];
        if let Some(t) = &params.r#type {
            q.push(("type".into(), t.clone()));
        }
        self.http.get_admin("/admin/fees/resolve", &q).await
    }

    // --- Agent Management ---

    pub async fn get_agent_status(&self, agent_id: &str) -> Result<AgentPaymentStatus> {
        let path = format!("/admin/agents/{}/status", encode(agent_id));
        self.http.get_admin(&path, &[]).await
    }

    pub async fn suspend_agent(
        &self,
        agent_id: &str,
        params: &SuspendAgentParams,
    ) -> Result<AgentPaymentStatus> {
        let path = format!("/admin/agents/{}/suspend", encode(agent_id));
        self.http.post_admin(&path, Some(params)).await
    }

    pub async fn unsuspend_agent(&self, agent_id: &str) -> Result<AgentPaymentStatus> {
        let path = format!("/admin/agents/{}/unsuspend", encode(agent_id));
        self.http
            .post_admin::<AgentPaymentStatus, serde_json::Value>(&path, None)
            .await
    }

    pub async fn flag_agent(
        &self,
        agent_id: &str,
        params: &serde_json::Value,
    ) -> Result<AgentPaymentStatus> {
        let path = format!("/admin/agents/{}/flag", encode(agent_id));
        self.http.post_admin(&path, Some(params)).await
    }

    // --- System Config ---

    pub async fn get_config(&self) -> Result<ConfigResponse> {
        self.http.get_admin("/admin/config", &[]).await
    }

    pub async fn set_config(
        &self,
        key: &str,
        value: &str,
        reason: Option<&str>,
    ) -> Result<SystemConfig> {
        let path = format!("/admin/config/{}", encode(key));
        let mut body = serde_json::Map::new();
        body.insert("value".into(), serde_json::Value::String(value.to_string()));
        if let Some(reason) = reason {
            body.insert(
                "reason".into(),
                serde_json::Value::String(reason.to_string()),
            );
        }
        let body = serde_json::Value::Object(body);
        self.http.put_admin(&path, Some(&body)).await
    }

    // --- Audit ---

    pub async fn audit(&self, params: Option<&AuditQueryParams>) -> Result<AuditResponse> {
        let mut q: Vec<(String, String)> = Vec::new();
        if let Some(p) = params {
            if let Some(v) = &p.actor {
                q.push(("actor".into(), v.clone()));
            }
            if let Some(v) = &p.action {
                q.push(("action".into(), v.clone()));
            }
            if let Some(v) = p.limit {
                q.push(("limit".into(), v.to_string()));
            }
            if let Some(v) = p.offset {
                q.push(("offset".into(), v.to_string()));
            }
        }
        self.http.get_admin("/admin/audit", &q).await
    }

    // --- Metrics ---

    pub async fn fee_metrics(&self, period: Option<&str>) -> Result<AdminFeeMetrics> {
        let mut q: Vec<(String, String)> = Vec::new();
        if let Some(v) = period {
            q.push(("period".into(), v.to_string()));
        }
        self.http.get_admin("/admin/metrics/fees", &q).await
    }
}
