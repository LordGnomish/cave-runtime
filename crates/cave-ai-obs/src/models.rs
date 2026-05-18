// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmTrace {
    pub id: Uuid,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub latency_ms: u64,
    pub cost_usd: f64,
    pub success: bool,
    pub created_at: DateTime<Utc>,
    pub tags: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmStats {
    pub model: String,
    pub total_requests: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub avg_latency_ms: f64,
    pub error_rate: f64,
}
