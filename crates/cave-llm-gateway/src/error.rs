// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error types for cave-llm-gateway.

use thiserror::Error;

pub type GatewayResult<T> = Result<T, GatewayError>;

#[derive(Error, Debug)]
pub enum GatewayError {
    #[error("No providers available")]
    NoProvidersAvailable,

    #[error("Provider '{provider}' unavailable: {reason}")]
    ProviderUnavailable { provider: String, reason: String },

    #[error("Upstream error (status={status}): {body}")]
    UpstreamError { status: u16, body: String },

    #[error("Provider not found: {0}")]
    ProviderNotFound(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Rate limit exceeded for consumer '{consumer}', retry after {retry_after_ms}ms")]
    RateLimitExceeded {
        consumer: String,
        retry_after_ms: u64,
    },

    #[error("Token budget exceeded: requested={requested}, budget={budget}")]
    TokenBudgetExceeded { requested: u32, budget: u32 },

    #[error("Budget exceeded for '{scope}': spent ${spent:.4} of ${limit:.4}")]
    BudgetExceeded {
        scope: String,
        spent: f64,
        limit: f64,
    },

    #[error("Blocked by guardrail '{rule}': {reason}")]
    GuardrailBlocked { rule: String, reason: String },

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Streaming error: {0}")]
    StreamingError(String),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("HTTP client error: {0}")]
    HttpClient(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl GatewayError {
    /// Map to an HTTP status code for API responses.
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Unauthorized(_) => 401,
            Self::GuardrailBlocked { .. } => 403,
            Self::ModelNotFound(_) | Self::ProviderNotFound(_) | Self::NotFound(_) => 404,
            Self::RateLimitExceeded { .. } => 429,
            Self::InvalidRequest(_) => 400,
            Self::TokenBudgetExceeded { .. } => 402,
            Self::BudgetExceeded { .. } => 402,
            Self::NoProvidersAvailable | Self::ProviderUnavailable { .. } => 503,
            _ => 500,
        }
    }
}
