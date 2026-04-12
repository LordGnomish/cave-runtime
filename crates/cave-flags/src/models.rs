//! Data models for cave-flags — full Unleash v6 API parity.
//!
//! Two model families coexist:
//! - **Unleash-compatible** types: `UnleashContext`, `FeatureToggle`, `StrategyConfig`, etc.
//! - **Legacy CAVE** types: `FeatureFlag`, `Strategy`, `EvaluationContext` (kept for backward compat)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ================================================================
// Unleash Evaluation Context
// ================================================================

/// Evaluation context as defined by the Unleash SDK protocol.
///
/// All fields are optional except for the standard ones; additional
/// runtime properties live in `properties`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnleashContext {
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub remote_address: Option<String>,
    pub current_time: Option<DateTime<Utc>>,
    pub environment: Option<String>,
    pub app_name: Option<String>,
    /// Custom context properties (e.g., "region", "plan", "hostname").
    #[serde(default)]
    pub properties: HashMap<String, String>,
}

// ================================================================
// Constraints
// ================================================================

/// Constraint operator types (Unleash constraint spec).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ConstraintOperator {
    #[serde(rename = "IN")]
    In,
    #[serde(rename = "NOT_IN")]
    NotIn,
    #[serde(rename = "STR_STARTS_WITH")]
    StrStartsWith,
    #[serde(rename = "STR_ENDS_WITH")]
    StrEndsWith,
    #[serde(rename = "STR_CONTAINS")]
    StrContains,
    #[serde(rename = "NUM_EQ")]
    NumEq,
    #[serde(rename = "NUM_GT")]
    NumGt,
    #[serde(rename = "NUM_GTE")]
    NumGte,
    #[serde(rename = "NUM_LT")]
    NumLt,
    #[serde(rename = "NUM_LTE")]
    NumLte,
    #[serde(rename = "DATE_BEFORE")]
    DateBefore,
    #[serde(rename = "DATE_AFTER")]
    DateAfter,
    #[serde(rename = "SEMVER_EQ")]
    SemverEq,
    #[serde(rename = "SEMVER_GT")]
    SemverGt,
    #[serde(rename = "SEMVER_LT")]
    SemverLt,
}

/// A constraint scoped to a single context field.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Constraint {
    /// Context field name (e.g. "userId", "environment", custom property key).
    pub context_name: String,
    pub operator: ConstraintOperator,
    /// Multi-value operators (IN, NOT_IN, STR_*).
    #[serde(default)]
    pub values: Vec<String>,
    /// Single-value operators (NUM_*, DATE_*, SEMVER_*).
    pub value: Option<String>,
    /// When true, the constraint result is negated after evaluation.
    #[serde(default)]
    pub inverted: bool,
    /// Case-insensitive matching for string operators.
    #[serde(default)]
    pub case_insensitive: bool,
}

// ================================================================
// Strategies
// ================================================================

/// Strategy configuration attached to a feature toggle.
///
/// `name` matches one of the built-in Unleash strategy names or a custom one.
/// `parameters` is the free-form key→value map each strategy reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyConfig {
    pub name: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    /// Segment IDs to expand and AND with `constraints`.
    #[serde(default)]
    pub segments: Vec<i64>,
}

// ================================================================
// Variants
// ================================================================

/// How variant weight is managed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeightType {
    /// Weight is adjusted automatically to fill remaining share.
    Variable,
    /// Weight is fixed regardless of other variants.
    Fix,
}

/// Payload delivered alongside a variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantPayload {
    /// One of: "string", "json", "csv", "number".
    #[serde(rename = "type")]
    pub payload_type: String,
    pub value: String,
}

/// Override: force this variant when a context field matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantOverride {
    pub context_name: String,
    pub values: Vec<String>,
}

/// A variant option on a feature toggle.
/// All variants on a toggle should sum to weight 1000.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variant {
    pub name: String,
    /// Weight out of 1000.
    pub weight: u32,
    #[serde(default = "default_weight_type")]
    pub weight_type: WeightType,
    pub payload: Option<VariantPayload>,
    #[serde(default)]
    pub overrides: Vec<VariantOverride>,
    /// Context field used for sticky assignment ("default", "userId", "sessionId", "random").
    #[serde(default = "default_stickiness")]
    pub stickiness: String,
}

fn default_weight_type() -> WeightType {
    WeightType::Variable
}
fn default_stickiness() -> String {
    "default".to_string()
}

/// Evaluated variant result returned to a client SDK.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantResult {
    pub name: String,
    pub enabled: bool,
    pub payload: Option<VariantPayload>,
    /// Whether the parent toggle itself is enabled.
    pub feature_enabled: bool,
}

impl VariantResult {
    pub fn disabled(feature_enabled: bool) -> Self {
        Self {
            name: "disabled".to_string(),
            enabled: false,
            payload: None,
            feature_enabled,
        }
    }
}

// ================================================================
// Feature Toggle
// ================================================================

/// A feature toggle — Unleash API-compatible representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureToggle {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    #[serde(default = "default_project")]
    pub project: String,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub impression_data: bool,
    #[serde(rename = "type", default = "default_toggle_type")]
    pub toggle_type: String,
    /// Active strategies (any match → enabled).
    #[serde(default)]
    pub strategies: Vec<StrategyConfig>,
    #[serde(default)]
    pub variants: Vec<Variant>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
    pub last_seen_at: Option<DateTime<Utc>>,
}

fn default_project() -> String {
    "default".to_string()
}
fn default_toggle_type() -> String {
    "release".to_string()
}

impl FeatureToggle {
    /// Create a new toggle with a single `default` strategy (always-on).
    pub fn new(name: impl Into<String>, created_by: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            name: name.into(),
            description: None,
            enabled: true,
            project: "default".to_string(),
            stale: false,
            archived: false,
            impression_data: false,
            toggle_type: "release".to_string(),
            strategies: vec![StrategyConfig {
                name: "default".to_string(),
                parameters: HashMap::new(),
                constraints: vec![],
                segments: vec![],
            }],
            variants: vec![],
            tags: vec![],
            created_at: now,
            updated_at: now,
            created_by: created_by.into(),
            last_seen_at: None,
        }
    }
}

// ================================================================
// Segment — reusable constraint groups
// ================================================================

/// A segment is a named group of constraints that strategies can reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

// ================================================================
// Project
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub health: i32,
    pub feature_count: i32,
    pub member_count: i32,
}

// ================================================================
// Tag
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    #[serde(rename = "type")]
    pub tag_type: String,
    pub value: String,
}

// ================================================================
// Event / Audit Log
// ================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: i64,
    #[serde(rename = "type")]
    pub event_type: String,
    pub created_by: String,
    pub data: Option<serde_json::Value>,
    pub pre_data: Option<serde_json::Value>,
    pub feature_name: Option<String>,
    pub project: Option<String>,
    pub environment: Option<String>,
    #[serde(default)]
    pub tags: Vec<Tag>,
    pub created_at: DateTime<Utc>,
}

// ================================================================
// Client SDK API Types
// ================================================================

/// Client SDK registration (POST /api/client/register).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientRegistration {
    pub app_name: String,
    pub instance_id: String,
    pub sdk_version: Option<String>,
    #[serde(default)]
    pub strategies: Vec<String>,
    pub started: DateTime<Utc>,
    pub interval: u64,
}

/// Client SDK metrics (POST /api/client/metrics).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientMetrics {
    pub app_name: String,
    pub instance_id: String,
    pub bucket: MetricsBucket,
}

#[derive(Debug, Deserialize)]
pub struct MetricsBucket {
    pub start: DateTime<Utc>,
    pub stop: DateTime<Utc>,
    pub toggles: HashMap<String, ToggleMetrics>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleMetrics {
    pub yes: u64,
    pub no: u64,
    #[serde(default)]
    pub variants: HashMap<String, u64>,
}

// ================================================================
// Admin API Request Types
// ================================================================

/// Create a new feature toggle (POST /api/admin/features).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateToggleRequest {
    pub name: String,
    pub description: Option<String>,
    pub project: Option<String>,
    #[serde(rename = "type")]
    pub toggle_type: Option<String>,
    #[serde(default)]
    pub impression_data: bool,
}

/// Update an existing feature toggle (PUT /api/admin/features/:name).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateToggleRequest {
    pub description: Option<String>,
    pub stale: Option<bool>,
    pub impression_data: Option<bool>,
}

/// Add a strategy to a feature toggle.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddStrategyRequest {
    pub name: String,
    #[serde(default)]
    pub parameters: HashMap<String, String>,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub segments: Vec<i64>,
}

// ================================================================
// Legacy CAVE Types — backward compat with original /api/flags/* API
// ================================================================

/// Original CAVE feature flag.  Used by the legacy /api/flags endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub flag_type: FlagType,
    pub strategy: Strategy,
    pub environments: Vec<String>,
    pub tenant_id: Option<String>,
    pub kill_switch: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagType {
    Boolean,
    Rollout,
    Variant,
    KillSwitch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Strategy {
    Default { enabled: bool },
    GradualRollout { percentage: u8, group_id: Option<String> },
    UserIds { user_ids: Vec<String> },
    TenantScope { tenant_ids: Vec<String> },
    EnvironmentScope { environments: Vec<String> },
    Custom { name: String, parameters: serde_json::Value },
}

/// Flag evaluation request — legacy CAVE API.
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

/// Flag evaluation response — legacy CAVE API.
#[derive(Debug, Serialize)]
pub struct EvaluateResponse {
    pub flags: Vec<FlagEvaluation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlagEvaluation {
    pub name: String,
    pub enabled: bool,
    pub variant: Option<String>,
}

/// Create flag request — legacy CAVE API.
#[derive(Debug, Deserialize)]
pub struct CreateFlagRequest {
    pub name: String,
    pub description: String,
    pub flag_type: FlagType,
    pub strategy: Strategy,
    pub environments: Vec<String>,
    pub tenant_id: Option<String>,
}
