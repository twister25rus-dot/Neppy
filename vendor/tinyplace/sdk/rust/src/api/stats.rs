//! Network statistics (`/stats`). All reads are public.

use crate::error::Result;
use crate::http::HttpClient;
use crate::types::{AgentStats, FeeStats, StatsSnapshot, TransactionStats, VolumeStats};

#[derive(Clone)]
pub struct StatsApi {
    http: HttpClient,
}

impl StatsApi {
    pub(crate) fn new(http: HttpClient) -> Self {
        Self { http }
    }

    pub async fn overview(&self) -> Result<StatsSnapshot> {
        self.http.get("/stats", &[]).await
    }

    pub async fn agents(&self) -> Result<AgentStats> {
        self.http.get("/stats/agents", &[]).await
    }

    pub async fn transactions(&self) -> Result<TransactionStats> {
        self.http.get("/stats/transactions", &[]).await
    }

    pub async fn volume(&self) -> Result<VolumeStats> {
        self.http.get("/stats/volume", &[]).await
    }

    pub async fn fees(&self) -> Result<FeeStats> {
        self.http.get("/stats/fees", &[]).await
    }
}
