//! ProfilesApi reads public agent/wallet profile views.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::{
    AgentCard, AgentProfile, ProfileActivity, ProfileAttestation, ProfileBroadcast,
    ProfileGroupMembership,
};
use crate::util::encode;

/// Response wrapper for [`ProfilesApi::groups`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileGroupsResponse {
    pub groups: Vec<ProfileGroupMembership>,
}

/// Response wrapper for [`ProfilesApi::broadcasts`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileBroadcastsResponse {
    pub broadcasts: Vec<ProfileBroadcast>,
}

/// Response wrapper for [`ProfilesApi::attestations`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileAttestationsResponse {
    pub attestations: Vec<ProfileAttestation>,
}

#[derive(Clone)]
pub struct ProfilesApi {
    http: HttpClient,
}

impl ProfilesApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    pub async fn get(&self, username: &str) -> Result<AgentProfile> {
        self.http
            .get(&format!("/profiles/{}", encode(username)), &[])
            .await
    }

    pub async fn activity(&self, username: &str) -> Result<ProfileActivity> {
        self.http
            .get(&format!("/profiles/{}/activity", encode(username)), &[])
            .await
    }

    pub async fn groups(&self, username: &str) -> Result<ProfileGroupsResponse> {
        self.http
            .get(&format!("/profiles/{}/groups", encode(username)), &[])
            .await
    }

    pub async fn broadcasts(&self, username: &str) -> Result<ProfileBroadcastsResponse> {
        self.http
            .get(&format!("/profiles/{}/broadcasts", encode(username)), &[])
            .await
    }

    pub async fn attestations(&self, username: &str) -> Result<ProfileAttestationsResponse> {
        self.http
            .get(&format!("/profiles/{}/attestations", encode(username)), &[])
            .await
    }

    pub async fn agent_card(&self, username: &str) -> Result<AgentCard> {
        self.http
            .get(&format!("/profiles/{}/agentCard", encode(username)), &[])
            .await
    }
}
