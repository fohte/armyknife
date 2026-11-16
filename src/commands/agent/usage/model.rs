use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::commands::agent::types::Engine;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionUsage {
    pub(crate) session_id: String,
    pub(crate) engine: Engine,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) context: Option<ContextUsage>,
    pub(crate) rate_limits: RateLimits,
    pub(crate) version: Option<String>,
    pub(crate) observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextUsage {
    pub(crate) used_tokens: u64,
    pub(crate) window_tokens: u64,
    pub(crate) used_percentage: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimits {
    pub(crate) five_hour: Option<RateLimitWindow>,
    pub(crate) seven_day: Option<RateLimitWindow>,
}

impl RateLimits {
    pub(crate) fn has_any(&self) -> bool {
        self.five_hour.is_some() || self.seven_day.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitWindow {
    pub(crate) used_percentage: f64,
    pub(crate) resets_at: Option<i64>,
    pub(crate) window_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EngineQuota {
    pub(crate) engine: Engine,
    pub(crate) rate_limits: RateLimits,
    pub(crate) observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EngineQuotas {
    pub(crate) claude: Option<EngineQuota>,
    pub(crate) codex: Option<EngineQuota>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageResponse {
    pub(crate) sessions: Vec<SessionUsage>,
    pub(crate) quotas: EngineQuotas,
}

impl EngineQuotas {
    pub(crate) fn set(&mut self, quota: EngineQuota) {
        match quota.engine {
            Engine::Claude => self.claude = Some(quota),
            Engine::Codex => self.codex = Some(quota),
        }
    }
}
