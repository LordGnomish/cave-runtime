// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shared types for the `/admin/litellm` page set. Mirrors
//! BerriAI/litellm v1.x proxy admin schemas (models, routes, API keys,
//! budgets, traffic stats).

use crate::admin::types::TenantId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmModel {
    pub tenant: TenantId,
    pub name: String,
    /// "openai" | "anthropic" | "cohere" | "azure" | "bedrock" | "vllm" | ...
    pub provider: String,
    pub model_id: String,
    /// "active" | "disabled"
    pub status: String,
    /// Maximum requests per minute (`rpm_limit`).
    pub rpm_limit: u32,
    /// Maximum tokens per minute.
    pub tpm_limit: u32,
    pub fallback_chain: Vec<String>,
    pub created_at_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmRoute {
    pub tenant: TenantId,
    pub name: String,
    /// Logical route pattern (`gpt-4*`, `claude-3*`, ...).
    pub pattern: String,
    pub target_models: Vec<String>,
    /// "round_robin" | "weighted" | "least_busy" | "lowest_cost"
    pub strategy: String,
    pub weights: Vec<(String, u32)>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmApiKey {
    pub tenant: TenantId,
    /// Public id; secret is held by cave-vault.
    pub key_id: String,
    pub label: String,
    pub allowed_models: Vec<String>,
    /// "active" | "revoked" | "expired"
    pub status: String,
    pub max_budget_usd_cents: Option<u64>,
    pub spent_usd_cents: u64,
    pub created_at_unix: i64,
    pub expires_at_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmBudget {
    pub tenant: TenantId,
    pub name: String,
    pub scope: String, // "tenant" | "team" | "key"
    pub limit_usd_cents: u64,
    pub spent_usd_cents: u64,
    pub period: String, // "monthly" | "weekly" | "daily"
    pub reset_at_unix: i64,
    pub alert_threshold_pct: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiteLlmTraffic {
    pub tenant: TenantId,
    pub model_name: String,
    pub window_seconds: u32,
    pub request_count: u64,
    pub error_count: u64,
    pub spend_usd_cents: u64,
    pub avg_latency_ms: u32,
    pub timeouts: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LiteLlmViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error("model {0} not found")]
    ModelNotFound(String),
    #[error("route {0} not found")]
    RouteNotFound(String),
    #[error("api key {0} not found")]
    KeyNotFound(String),
}
