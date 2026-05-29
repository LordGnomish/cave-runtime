// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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

/// Provider that served an LLM request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    OpenAi,
    Anthropic,
    Mistral,
    Ollama,
    Other,
}

impl Default for LlmProvider {
    fn default() -> Self {
        LlmProvider::Other
    }
}

/// Outcome of an LLM request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Success,
    Error,
    Timeout,
}

impl Default for RequestStatus {
    fn default() -> Self {
        RequestStatus::Success
    }
}

/// A single recorded LLM request (the unit of analytics in [`crate::store::AiObsStore`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub id: Uuid,
    pub provider: LlmProvider,
    pub model: String,
    pub user_id: Option<String>,
    pub status: RequestStatus,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub created_at: DateTime<Utc>,
}

/// Aggregate token totals/averages across a set of requests.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TokenStats {
    pub total_prompt: u64,
    pub total_completion: u64,
    pub total: u64,
    pub avg_prompt_per_request: f64,
    pub avg_completion_per_request: f64,
}

/// Aggregate cost totals and breakdowns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostStats {
    pub total_usd: f64,
    pub by_model: std::collections::HashMap<String, f64>,
    pub by_provider: std::collections::HashMap<String, f64>,
    pub by_user: std::collections::HashMap<String, f64>,
    pub avg_per_request: f64,
}

/// Latency distribution summary (milliseconds).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LatencyStats {
    pub avg_ms: f64,
    pub p50_ms: u64,
    pub p95_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
}

/// Top-level analytics snapshot for the AI-observability store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiObsStats {
    pub request_count: u64,
    pub success_rate: f64,
    pub token_stats: TokenStats,
    pub cost_stats: CostStats,
    pub latency_stats: LatencyStats,
    pub error_rate_by_model: std::collections::HashMap<String, f64>,
}
