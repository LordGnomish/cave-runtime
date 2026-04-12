//! Data models for CAVE LLM Gateway.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Supported LLM providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    OpenAi,
    Anthropic,
    AzureOpenAi,
    GoogleVertexAi,
    Local,
}

/// A model alias mapping a friendly name to a specific provider + model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAlias {
    pub alias: String,
    pub provider: Provider,
    pub model_id: String,
    pub max_tokens: Option<u32>,
    pub pinned_version: Option<String>,
}

/// A single message in a chat conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    /// "system", "user", or "assistant"
    pub role: String,
    pub content: String,
}

/// OpenAI-compatible chat completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: Option<bool>,
    pub user: Option<String>,
    /// Fallback providers/models if the primary fails.
    #[serde(default)]
    pub cave_fallback: Vec<String>,
    /// Force routing to a specific provider.
    #[serde(default)]
    pub cave_route_to: Option<String>,
}

/// Token usage counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// A single choice in a chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: String,
}

/// OpenAI-compatible chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatCompletionChoice>,
    pub usage: Usage,
}

/// Legacy text completion request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub model: String,
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

/// A single choice in a legacy completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionChoice {
    pub text: String,
    pub index: u32,
    pub finish_reason: String,
}

/// Legacy text completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<CompletionChoice>,
}

/// Either a single string or a list of strings for embedding input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringOrArray {
    Single(String),
    Multiple(Vec<String>),
}

/// Embedding request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: StringOrArray,
}

/// A single embedding object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingObject {
    pub index: u32,
    pub embedding: Vec<f32>,
    pub object: String,
}

/// Embedding response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    pub object: String,
    pub data: Vec<EmbeddingObject>,
    pub model: String,
    pub usage: Usage,
}

/// A record of tokens consumed and cost for a single request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendRecord {
    pub id: Uuid,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    pub provider: Provider,
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cost_usd: f64,
    pub request_id: String,
    pub created_at: DateTime<Utc>,
}

/// The kind of entity a budget limit applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetEntityType {
    User,
    Team,
    Project,
}

/// Time period over which a budget resets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Daily,
    Weekly,
    Monthly,
}

/// A spending budget for a user, team, or project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLimit {
    pub id: Uuid,
    pub entity_type: BudgetEntityType,
    pub entity_id: String,
    pub limit_usd: f64,
    pub period: BudgetPeriod,
    pub current_spend: f64,
    pub reset_at: DateTime<Utc>,
}

/// The kind of entity a rate limit applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitEntityType {
    User,
    Provider,
}

/// Per-entity rate limit state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimit {
    pub entity_type: RateLimitEntityType,
    pub entity_id: String,
    pub requests_per_minute: u32,
    pub tokens_per_minute: u32,
    pub current_requests: u32,
    pub current_tokens: u32,
    pub window_start: DateTime<Utc>,
}

/// Configuration for a single upstream provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub provider: Provider,
    pub api_base_url: String,
    pub api_key_env_var: String,
    pub enabled: bool,
    /// Relative weight for load balancing.
    pub weight: u32,
    pub max_retries: u32,
    pub timeout_seconds: u64,
    pub models: Vec<String>,
}

/// A log entry for a single gateway request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLog {
    pub id: Uuid,
    pub request_id: String,
    pub user_id: Option<String>,
    pub provider: Provider,
    pub model: String,
    pub request_type: String,
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub latency_ms: u64,
    pub status_code: u16,
    pub error: Option<String>,
    pub was_cached: bool,
    pub created_at: DateTime<Utc>,
}

/// A cached response entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub response: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub hit_count: u32,
    pub model: String,
}

/// Overall result from running guardrails on a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailResult {
    pub passed: bool,
    pub violations: Vec<GuardrailViolation>,
}

/// A single guardrail violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardrailViolation {
    pub rule_type: GuardrailType,
    pub description: String,
    pub severity: String,
}

/// Categories of guardrail checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailType {
    PiiDetected,
    ContentFiltered,
    PromptInjection,
    ToxicContent,
}
