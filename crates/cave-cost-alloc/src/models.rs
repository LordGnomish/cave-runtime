// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A cost center representing a team, project, or department.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostCenter {
    pub id: Uuid,
    pub name: String,
    pub team: String,
    pub project: String,
    pub department: String,
    /// Monthly budget in USD.
    pub budget_usd: f64,
    pub owner_email: String,
    pub tags: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Maps a cloud/cluster resource to a cost center with an optional split percentage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAllocation {
    pub id: Uuid,
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub cost_center_id: Uuid,
    /// Percentage of the resource's cost attributed here (0.0–100.0).
    pub split_percentage: f64,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    KubernetesPod,
    KubernetesNode,
    StorageVolume,
    LoadBalancer,
    Database,
    Network,
    Compute,
    Other,
}

/// Period-based cost breakdown for a cost center / environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostReport {
    pub id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub cost_center_id: Uuid,
    pub environment: String,
    pub total_cost_usd: f64,
    pub breakdown: Vec<CostLineItem>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostLineItem {
    pub resource_type: ResourceType,
    pub description: String,
    pub quantity: f64,
    pub unit_price_usd: f64,
    pub total_usd: f64,
}

/// Spending limits, alert thresholds, and auto-scaling caps for a cost center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetPolicy {
    pub id: Uuid,
    pub cost_center_id: Uuid,
    pub period: BudgetPeriod,
    pub limit_usd: f64,
    /// Alert fires when spend reaches this percentage of the limit.
    pub alert_threshold_pct: f64,
    /// When true, spending is hard-blocked beyond `limit_usd`.
    pub hard_cap: bool,
    /// Auto-scaling is capped at this percentage of the budget.
    pub auto_scale_cap_pct: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Monthly,
    Quarterly,
    Annual,
}

/// Defines how shared infrastructure costs are split across cost centers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargebackRule {
    pub id: Uuid,
    pub name: String,
    pub resource_type: ResourceType,
    pub split_strategy: SplitStrategy,
    pub cost_center_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SplitStrategy {
    ByCpu,
    ByMemory,
    ByRequestCount,
    Equal,
    /// Explicit per-cost-center weights (cost center ID → weight).
    ByCustomWeights { weights: HashMap<String, f64> },
}

/// Showback report — awareness-only, no actual billing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowbackReport {
    pub id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub cost_center_id: Uuid,
    pub team: String,
    pub actual_cost_usd: f64,
    pub showback_cost_usd: f64,
    pub savings_opportunities: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

/// Linear trend-based spending forecast for a cost center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastModel {
    pub id: Uuid,
    pub cost_center_id: Uuid,
    pub forecast_months: u32,
    /// Linear trend: predicted_cost ≈ slope * t + intercept
    pub trend_slope: f64,
    pub trend_intercept: f64,
    pub forecast_points: Vec<ForecastPoint>,
    /// Confidence level in [0.0, 1.0].
    pub confidence: f64,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastPoint {
    pub month: String,
    pub predicted_cost_usd: f64,
    pub lower_bound_usd: f64,
    pub upper_bound_usd: f64,
}

/// Unusual spending spike detected for a resource / cost center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAnomaly {
    pub id: Uuid,
    pub cost_center_id: Uuid,
    pub resource_id: String,
    pub detected_at: DateTime<Utc>,
    pub expected_cost_usd: f64,
    pub actual_cost_usd: f64,
    pub deviation_pct: f64,
    pub severity: AnomalySeverity,
    pub status: AnomalyStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalySeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnomalyStatus {
    Open,
    Acknowledged,
    Resolved,
}

/// Required tags for cost attribution (team, project, environment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagPolicy {
    pub id: Uuid,
    pub name: String,
    pub required_tags: Vec<String>,
    pub resource_types: Vec<ResourceType>,
    pub enforcement: TagEnforcement,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagEnforcement {
    Warn,
    Block,
}

/// Generated chargeback invoice for a cost center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub cost_center_id: Uuid,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub line_items: Vec<InvoiceLineItem>,
    pub total_usd: f64,
    pub status: InvoiceStatus,
    pub issued_at: Option<DateTime<Utc>>,
    pub due_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLineItem {
    pub description: String,
    pub resource_type: ResourceType,
    pub quantity: f64,
    pub unit_price_usd: f64,
    pub total_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    Issued,
    Paid,
}

// --- Derived / computed types ---

/// Budget compliance result for a cost center.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetComplianceEntry {
    pub cost_center_id: Uuid,
    pub cost_center_name: String,
    pub budget_limit_usd: f64,
    pub current_spend_usd: f64,
    pub utilization_pct: f64,
    pub status: ComplianceStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceStatus {
    Healthy,
    Warning,
    Over,
}

/// An idle or under-utilized resource that is wasting money.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdleResource {
    pub resource_id: String,
    pub resource_type: ResourceType,
    pub cost_center_id: Option<Uuid>,
    pub utilization_pct: f64,
    /// Estimated monthly waste in USD.
    pub wasted_cost_usd: f64,
    pub recommendation: String,
}

/// Platform-level unit economics (cost per request/user/deployment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitEconomics {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub cost_per_request_usd: f64,
    pub cost_per_user_usd: f64,
    pub cost_per_deployment_usd: f64,
    pub total_requests: u64,
    pub total_users: u64,
    pub total_deployments: u64,
    pub total_cost_usd: f64,
}

// --- Request DTOs ---

#[derive(Debug, Deserialize)]
pub struct CreateCostCenterRequest {
    pub name: String,
    pub team: String,
    pub project: String,
    pub department: String,
    pub budget_usd: f64,
    pub owner_email: String,
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBudgetPolicyRequest {
    pub cost_center_id: Uuid,
    pub period: BudgetPeriod,
    pub limit_usd: f64,
    pub alert_threshold_pct: f64,
    pub hard_cap: bool,
    pub auto_scale_cap_pct: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreateChargebackRuleRequest {
    pub name: String,
    pub resource_type: ResourceType,
    pub split_strategy: SplitStrategy,
    pub cost_center_ids: Vec<Uuid>,
}

// --- Query params ---

#[derive(Debug, Deserialize)]
pub struct ShowbackQuery {
    pub period: Option<String>,
    pub team: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChargebackQuery {
    pub period: Option<String>,
    pub team: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ForecastQuery {
    pub months: Option<u32>,
}
