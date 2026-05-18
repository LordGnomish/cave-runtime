// SPDX-License-Identifier: AGPL-3.0-or-later
//! Unleash-compatible data models for feature flags.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Feature flag ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlag {
    pub name: String,
    #[serde(rename = "type")]
    pub feature_type: FeatureType,
    pub description: String,
    pub enabled: bool,
    pub stale: bool,
    #[serde(rename = "impressionData")]
    pub impression_data: bool,
    pub project: String,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
    pub strategies: Vec<FeatureStrategy>,
    pub variants: Vec<Variant>,
    pub environments: Vec<FeatureEnvironment>,
    pub tags: Vec<Tag>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureType {
    Release,
    Experiment,
    Operational,
    #[serde(rename = "kill-switch")]
    KillSwitch,
    Permission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureEnvironment {
    pub name: String,
    pub enabled: bool,
    pub strategies: Vec<FeatureStrategy>,
    pub variants: Vec<Variant>,
}

// ── Strategies ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureStrategy {
    pub id: Uuid,
    pub name: String,
    pub parameters: HashMap<String, String>,
    pub constraints: Vec<Constraint>,
    pub segments: Vec<i64>,
    #[serde(rename = "sortOrder")]
    pub sort_order: i32,
    pub disabled: bool,
    pub variants: Vec<StrategyVariant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub name: String,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Vec<StrategyParameter>,
    pub built_in: bool,
    pub deprecated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub description: String,
    pub required: bool,
}

// ── Constraints ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Constraint {
    #[serde(rename = "contextName")]
    pub context_name: String,
    pub operator: ConstraintOperator,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default)]
    pub inverted: bool,
    #[serde(rename = "caseInsensitive", default)]
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConstraintOperator {
    In,
    NotIn,
    StrStartsWith,
    StrEndsWith,
    StrContains,
    NumEq,
    NumGt,
    NumGte,
    NumLt,
    NumLte,
    DateBefore,
    DateAfter,
    SemverEq,
    SemverGt,
    SemverLt,
}

// ── Segments ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub constraints: Vec<Constraint>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

// ── Variants ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub name: String,
    pub weight: u32,
    #[serde(rename = "weightType")]
    pub weight_type: WeightType,
    pub stickiness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<VariantPayload>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<VariantOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeightType {
    Variable,
    Fix,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantPayload {
    #[serde(rename = "type")]
    pub payload_type: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantOverride {
    #[serde(rename = "contextName")]
    pub context_name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyVariant {
    pub name: String,
    pub weight: u32,
    #[serde(rename = "weightType")]
    pub weight_type: WeightType,
    pub stickiness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<VariantPayload>,
}

// ── Projects ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(rename = "defaultStickiness")]
    pub default_stickiness: String,
    pub mode: ProjectMode,
    pub members: i64,
    pub health: i64,
    pub feature_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectMode {
    Open,
    Protected,
    Private,
}

// ── Environments ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Environment {
    pub name: String,
    #[serde(rename = "type")]
    pub env_type: String,
    pub enabled: bool,
    pub protected: bool,
    pub sort_order: i32,
}

// ── API Tokens ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub secret: String,
    pub username: String,
    #[serde(rename = "type")]
    pub token_type: TokenType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub projects: Vec<String>,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenType {
    Client,
    Frontend,
    Admin,
}

// ── Tags ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    #[serde(rename = "type")]
    pub tag_type: String,
    pub value: String,
}

// ── Context fields ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextField {
    pub name: String,
    pub description: String,
    #[serde(rename = "legalValues")]
    pub legal_values: Vec<LegalValue>,
    #[serde(rename = "stickiness")]
    pub used_for_stickiness: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegalValue {
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Change requests ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRequest {
    pub id: i64,
    pub title: String,
    pub state: ChangeRequestState,
    pub project: String,
    pub environment: String,
    pub min_approvals: i32,
    pub approvals: Vec<ChangeRequestApproval>,
    pub rejections: Vec<ChangeRequestApproval>,
    pub changes: Vec<FeatureChange>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeRequestState {
    Draft,
    InReview,
    Approved,
    Applied,
    Rejected,
    Cancelled,
    Scheduled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRequestApproval {
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureChange {
    pub feature: String,
    pub action: String,
    pub payload: serde_json::Value,
}

// ── Banners ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Banner {
    pub id: i64,
    pub message: String,
    pub variant: BannerVariant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(rename = "linkText", skip_serializing_if = "Option::is_none")]
    pub link_text: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BannerVariant {
    Info,
    Warning,
    Error,
    Success,
}

// ── Metrics ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct MetricsReport {
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(rename = "instanceId")]
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
    pub variants: Option<HashMap<String, u64>>,
}

// ── SDK Registration ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SdkRegistration {
    #[serde(rename = "appName")]
    pub app_name: String,
    #[serde(rename = "instanceId")]
    pub instance_id: String,
    #[serde(rename = "sdkVersion")]
    pub sdk_version: Option<String>,
    pub strategies: Vec<String>,
    pub started: DateTime<Utc>,
    pub interval: u64,
}

// ── Evaluation context ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UnleashContext {
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
    #[serde(rename = "remoteAddress")]
    pub remote_address: Option<String>,
    pub environment: Option<String>,
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    #[serde(rename = "currentTime")]
    pub current_time: Option<String>,
    #[serde(default)]
    pub properties: HashMap<String, String>,
}

impl UnleashContext {
    pub fn get_field(&self, name: &str) -> Option<String> {
        match name {
            "userId" => self.user_id.clone(),
            "sessionId" => self.session_id.clone(),
            "remoteAddress" => self.remote_address.clone(),
            "environment" => self.environment.clone(),
            "appName" => self.app_name.clone(),
            "currentTime" => self.current_time.clone(),
            _ => self.properties.get(name).cloned(),
        }
    }
}

// ── Client API response ───────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ClientFeaturesResponse {
    pub version: u8,
    pub features: Vec<ClientFeature>,
    pub segments: Vec<Segment>,
    pub query: ClientQuery,
}

#[derive(Debug, Serialize)]
pub struct ClientFeature {
    pub name: String,
    #[serde(rename = "type")]
    pub feature_type: FeatureType,
    pub enabled: bool,
    pub stale: bool,
    pub strategies: Vec<FeatureStrategy>,
    pub variants: Vec<Variant>,
    #[serde(rename = "impressionData")]
    pub impression_data: bool,
    #[serde(rename = "lastSeenAt")]
    pub last_seen_at: Option<DateTime<Utc>>,
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ClientQuery {
    #[serde(rename = "inlineSegmentConstraints")]
    pub inline_segment_constraints: bool,
}

impl Default for ClientQuery {
    fn default() -> Self {
        Self { inline_segment_constraints: true }
    }
}

#[derive(Debug, Serialize)]
pub struct FrontendFeaturesResponse {
    pub toggles: Vec<FrontendToggle>,
}

#[derive(Debug, Serialize)]
pub struct FrontendToggle {
    pub name: String,
    pub enabled: bool,
    pub variant: EvaluatedVariant,
    #[serde(rename = "impressionData")]
    pub impression_data: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluatedVariant {
    pub name: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<VariantPayload>,
    #[serde(rename = "featureEnabled")]
    pub feature_enabled: bool,
}

impl EvaluatedVariant {
    pub fn disabled() -> Self {
        Self {
            name: "disabled".to_string(),
            enabled: false,
            payload: None,
            feature_enabled: false,
        }
    }
}

// ── Admin API request types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateFeatureRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub feature_type: Option<FeatureType>,
    #[serde(rename = "impressionData")]
    pub impression_data: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFeatureRequest {
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub feature_type: Option<FeatureType>,
    pub stale: Option<bool>,
    #[serde(rename = "impressionData")]
    pub impression_data: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct AddStrategyRequest {
    pub name: String,
    pub parameters: Option<HashMap<String, String>>,
    pub constraints: Option<Vec<Constraint>>,
    pub segments: Option<Vec<i64>>,
    pub variants: Option<Vec<StrategyVariant>>,
    #[serde(rename = "sortOrder")]
    pub sort_order: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSegmentRequest {
    pub name: String,
    pub description: Option<String>,
    pub constraints: Vec<Constraint>,
    pub project: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiTokenRequest {
    pub username: String,
    #[serde(rename = "type")]
    pub token_type: TokenType,
    pub environment: Option<String>,
    pub projects: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBannerRequest {
    pub message: String,
    pub variant: BannerVariant,
    pub link: Option<String>,
    pub link_text: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChangeRequestRequest {
    pub title: Option<String>,
    pub environment: String,
    pub min_approvals: Option<i32>,
}
