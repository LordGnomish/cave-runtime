//! Data models for cave-llm-gateway.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Provider ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Ollama,
    OpenAI,
    Anthropic,
    AzureOpenAI,
    Bedrock,
    Google,
    Mistral,
    Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProvider {
    pub id: Uuid,
    /// Human-readable name (e.g. "prod-openai", "local-ollama").
    pub name: String,
    pub provider_type: ProviderType,
    pub endpoint: String,
    /// Reference key into cave-vault for the API credential.
    pub api_key_ref: Option<String>,
    pub models_available: Vec<String>,
    pub health_status: HealthStatus,
    /// Lower = higher priority in PriorityFallback strategy.
    pub priority: u32,
    /// Relative weight for weighted round-robin / load balancing.
    pub weight: f64,
}

// ─── Request / Response ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestMetadata {
    pub user_id: Option<String>,
    pub project: Option<String>,
    pub team: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
    /// Routing / billing metadata — not forwarded to providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<RequestMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Choice {
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub id: Uuid,
    pub model: String,
    pub provider_used: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
    pub latency_ms: u64,
    /// Estimated cost in USD, if calculable.
    pub cost: Option<f64>,
    pub cached: bool,
}

// ─── Routing ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    PriorityFallback,
    RoundRobin,
    LeastLatency,
    CostOptimized,
    Custom,
}

impl Default for RoutingStrategy {
    fn default() -> Self {
        Self::PriorityFallback
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay_ms: 500,
            backoff_multiplier: 2.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    pub id: Uuid,
    pub name: String,
    pub strategy: RoutingStrategy,
    /// Provider IDs to try in order when the primary fails.
    pub fallback_chain: Vec<Uuid>,
    pub retry_config: RetryConfig,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "default".to_string(),
            strategy: RoutingStrategy::default(),
            fallback_chain: Vec::new(),
            retry_config: RetryConfig::default(),
        }
    }
}

/// Alias → actual model name per provider.
/// e.g. "gpt-4" → OpenAI:"gpt-4-turbo", Anthropic:"claude-sonnet-4-20250514"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMapping {
    pub alias: String,
    pub provider_models: HashMap<ProviderType, String>,
}

// ─── Budget ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScope {
    Team,
    Project,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub id: Uuid,
    pub scope: BudgetScope,
    pub scope_id: String,
    pub period: BudgetPeriod,
    /// Maximum tokens allowed per period.
    pub limit: u64,
    pub current_usage: u64,
    /// Fraction [0.0, 1.0] at which to emit an alert (e.g. 0.8 = 80%).
    pub alert_threshold: f64,
    pub period_start: DateTime<Utc>,
}

// ─── Semantic Cache ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticCacheEntry {
    pub prompt_hash: String,
    pub response: LlmResponse,
    pub created_at: DateTime<Utc>,
    pub ttl_seconds: u64,
    pub hit_count: u64,
    pub model: String,
}

impl SemanticCacheEntry {
    pub fn is_fresh(&self) -> bool {
        let age = Utc::now()
            .signed_duration_since(self.created_at)
            .num_seconds();
        age >= 0 && (age as u64) < self.ttl_seconds
    }
}

// ─── Guardrails ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailType {
    PiiFilter,
    ContentPolicy,
    TokenLimit,
    CostLimit,
    RateLimit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailAction {
    Block,
    Warn,
    Redact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guardrail {
    pub id: Uuid,
    pub name: String,
    pub guardrail_type: GuardrailType,
    /// Free-form JSON config consumed by the guardrail implementation.
    pub config: serde_json::Value,
    pub action: GuardrailAction,
    pub enabled: bool,
}

// ─── Provider Health ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider_id: Uuid,
    pub latency_p50_ms: f64,
    pub latency_p95_ms: f64,
    pub latency_p99_ms: f64,
    pub error_rate: f64,
    pub last_check: DateTime<Utc>,
    pub status: HealthStatus,
}
