//! Moderation: constitution, reports, actions, and appeals.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::{
    Constitution, ModerationAction, ModerationAppeal, ModerationReport, ModerationReportCreate,
};
use crate::util::encode;

/// Body for [`ModerationApi::create_appeal`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModerationAppealCreate {
    pub action_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub comment: Option<String>,
}

/// Body for status updates on reports and appeals.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModerationStatusUpdate {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModerationActionsResponse {
    pub actions: Vec<ModerationAction>,
}

/// Filters for [`ModerationApi::list_actions`].
#[derive(Debug, Clone, Default)]
pub struct ModerationActionsQueryParams {
    pub target: Option<String>,
    pub action_type: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Clone)]
pub struct ModerationApi {
    http: HttpClient,
}

impl ModerationApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    pub async fn get_constitution(&self) -> Result<Constitution> {
        self.http.get("/constitution", &[]).await
    }

    pub async fn create_report(&self, report: &ModerationReportCreate) -> Result<ModerationReport> {
        let mut report = report.clone();
        if report.report_id.is_none() {
            report.report_id = Some(next_client_id("report"));
        }
        self.http
            .post_directory_auth_as(
                "/moderation/reports",
                &report.reporter.clone(),
                Some(&report),
            )
            .await
    }

    pub async fn get_report(&self, report_id: &str) -> Result<ModerationReport> {
        let path = format!("/moderation/reports/{}", encode(report_id));
        self.http.get_auth(&path, &[]).await
    }

    pub async fn update_report_status(
        &self,
        report_id: &str,
        update: &ModerationStatusUpdate,
    ) -> Result<ModerationReport> {
        let path = format!("/moderation/reports/{}/status", encode(report_id));
        self.http.put_directory_auth(&path, Some(update)).await
    }

    pub async fn list_actions(
        &self,
        params: Option<&ModerationActionsQueryParams>,
    ) -> Result<ModerationActionsResponse> {
        let mut q: Vec<(String, String)> = Vec::new();
        if let Some(p) = params {
            if let Some(v) = &p.target {
                q.push(("target".into(), v.clone()));
            }
            if let Some(v) = &p.action_type {
                q.push(("type".into(), v.clone()));
            }
            if let Some(v) = p.limit {
                q.push(("limit".into(), v.to_string()));
            }
            if let Some(v) = p.offset {
                q.push(("offset".into(), v.to_string()));
            }
        }
        self.http.get("/moderation/actions", &q).await
    }

    pub async fn create_action(&self, action: &ModerationAction) -> Result<ModerationAction> {
        self.http
            .post_directory_auth("/moderation/actions", Some(action))
            .await
    }

    pub async fn create_appeal(
        &self,
        appeal: &ModerationAppealCreate,
        appellant: Option<&str>,
    ) -> Result<ModerationAppeal> {
        match appellant {
            Some(actor) => {
                self.http
                    .post_directory_auth_as("/moderation/appeals", actor, Some(appeal))
                    .await
            }
            None => {
                self.http
                    .post_directory_auth("/moderation/appeals", Some(appeal))
                    .await
            }
        }
    }

    pub async fn get_appeal(&self, appeal_id: &str) -> Result<ModerationAppeal> {
        let path = format!("/moderation/appeals/{}", encode(appeal_id));
        self.http.get_auth(&path, &[]).await
    }

    pub async fn update_appeal_status(
        &self,
        appeal_id: &str,
        update: &ModerationStatusUpdate,
    ) -> Result<ModerationAppeal> {
        let path = format!("/moderation/appeals/{}/status", encode(appeal_id));
        self.http.put_directory_auth(&path, Some(update)).await
    }
}

fn next_client_id(prefix: &str) -> String {
    use rand::RngCore as _;
    let mut random = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{prefix}_{}_{suffix}", radix36(now_ms))
}

fn radix36(mut value: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii digits")
}
