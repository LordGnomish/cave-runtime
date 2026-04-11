//! Data models for feature flags.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A feature flag definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub flag_type: FlagType,
    pub strategy: Strategy,
    /// Environment scope (e.g., "dev", "staging", "prod")
    pub environments: Vec<String>,
    /// Tenant scope (empty = platform-wide)
    pub tenant_id: Option<String>,
    /// Kill switch — overrides all strategies when true
    pub kill_switch: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid, // cave_uid
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagType {
    /// Simple on/off
    Boolean,
    /// Gradual rollout by percentage
    Rollout,
    /// A/B testing with variants
    Variant,
    /// Kill switch (emergency disable)
    KillSwitch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Strategy {
    /// Always on/off
    Default { enabled: bool },
    /// Percentage-based gradual rollout
    GradualRollout { percentage: u8, group_id: Option<String> },
    /// User ID allowlist
    UserIds { user_ids: Vec<String> },
    /// Tenant-scoped
    TenantScope { tenant_ids: Vec<String> },
    /// Environment-scoped
    EnvironmentScope { environments: Vec<String> },
    /// Custom strategy (extensible)
    Custom { name: String, parameters: serde_json::Value },
}

/// Flag evaluation request (from client SDKs).
#[derive(Debug, Deserialize)]
pub struct EvaluateRequest {
    pub context: EvaluationContext,
}

#[derive(Debug, Deserialize)]
pub struct EvaluationContext {
    pub user_id: Option<String>,
    pub tenant_id: Option<String>,
    pub environment: String,
    pub properties: Option<serde_json::Value>,
}

/// Flag evaluation response.
#[derive(Debug, Serialize)]
pub struct EvaluateResponse {
    pub flags: Vec<FlagEvaluation>,
}

#[derive(Debug, Serialize)]
pub struct FlagEvaluation {
    pub name: String,
    pub enabled: bool,
    pub variant: Option<String>,
}

/// Create flag request.
#[derive(Debug, Deserialize)]
pub struct CreateFlagRequest {
    pub name: String,
    pub description: String,
    pub flag_type: FlagType,
    pub strategy: Strategy,
    pub environments: Vec<String>,
    pub tenant_id: Option<String>,
}
