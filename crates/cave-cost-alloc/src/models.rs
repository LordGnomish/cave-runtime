//! Data models for cave-cost-alloc.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ─── Resource types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Cpu,
    Memory,
    Gpu,
    Storage,
    Network,
    LoadBalancer,
}

// ─── Cost rates (AWS us-east-1 on-demand approximate) ────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRate {
    pub cpu_per_core_hour: f64,
    pub memory_per_gb_hour: f64,
    pub gpu_per_hour: f64,
    pub storage_per_gb_month: f64,
    pub network_egress_per_gb: f64,
    pub load_balancer_per_hour: f64,
}

// ─── Resource usage ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub cpu_cores: f64,
    pub memory_gb: f64,
    pub gpu_count: u32,
    pub storage_gb: f64,
    pub network_egress_gb: f64,
    pub load_balancers: u32,
}

// ─── Resource cost ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceCost {
    pub cpu_cost: f64,
    pub memory_cost: f64,
    pub gpu_cost: f64,
    pub storage_cost: f64,
    pub network_cost: f64,
    pub lb_cost: f64,
    pub total_cost: f64,
}

impl ResourceCost {
    pub fn add(&self, other: &ResourceCost) -> ResourceCost {
        ResourceCost {
            cpu_cost: self.cpu_cost + other.cpu_cost,
            memory_cost: self.memory_cost + other.memory_cost,
            gpu_cost: self.gpu_cost + other.gpu_cost,
            storage_cost: self.storage_cost + other.storage_cost,
            network_cost: self.network_cost + other.network_cost,
            lb_cost: self.lb_cost + other.lb_cost,
            total_cost: self.total_cost + other.total_cost,
        }
    }
}

// ─── Allocation dimension ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationDimension {
    Namespace,
    Label(String),
    Deployment,
    Pod,
    Container,
}

// ─── Cost allocation ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAllocation {
    pub id: Uuid,
    pub namespace: String,
    pub deployment: Option<String>,
    pub pod: Option<String>,
    pub container: Option<String>,
    pub labels: HashMap<String, String>,
    pub team: Option<String>,
    pub cost_center: Option<String>,
    pub usage: ResourceUsage,
    pub cost: ResourceCost,
    /// Actual / requested ratio: 0.0–1.0
    pub efficiency_score: f32,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

// ─── Budget ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetPeriod {
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAlert {
    pub id: Uuid,
    pub name: String,
    pub namespace: Option<String>,
    pub team: Option<String>,
    pub cost_center: Option<String>,
    pub threshold_usd: f64,
    pub period: BudgetPeriod,
    pub current_spend: f64,
    pub alert_fired: bool,
    pub created_at: DateTime<Utc>,
}

// ─── Efficiency report ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EfficiencyReport {
    pub namespace: String,
    pub cpu_requested: f64,
    pub cpu_used: f64,
    pub memory_requested_gb: f64,
    pub memory_used_gb: f64,
    pub cpu_efficiency: f32,
    pub memory_efficiency: f32,
    pub overall_efficiency: f32,
}

// ─── Recommendation ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationType {
    RightSizeCpu,
    RightSizeMemory,
    UseSpotInstances,
    DeleteUnused,
    ConsolidateNodes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub id: Uuid,
    pub resource_type: String,
    pub namespace: String,
    pub deployment: Option<String>,
    pub recommendation_type: RecommendationType,
    pub current_config: serde_json::Value,
    pub recommended_config: serde_json::Value,
    pub estimated_savings_usd_monthly: f64,
    pub confidence: f32,
    pub created_at: DateTime<Utc>,
}

// ─── Cloud cost entry ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    Aws,
    Azure,
    Gcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudCostEntry {
    pub id: Uuid,
    pub provider: CloudProvider,
    pub account_id: String,
    pub service: String,
    pub region: String,
    pub resource_id: Option<String>,
    pub tags: HashMap<String, String>,
    pub cost_usd: f64,
    pub usage_quantity: f64,
    pub usage_unit: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

// ─── Showback / Chargeback reports ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowbackLineItem {
    pub group: String,
    pub cost: ResourceCost,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowbackReport {
    pub group_by: String,
    pub period_start: Option<DateTime<Utc>>,
    pub period_end: Option<DateTime<Utc>>,
    pub line_items: Vec<ShowbackLineItem>,
    pub total_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargebackLineItem {
    pub cost_center: String,
    pub team: Option<String>,
    pub cost: ResourceCost,
    pub allocation_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChargebackReport {
    pub period_start: Option<DateTime<Utc>>,
    pub period_end: Option<DateTime<Utc>>,
    pub line_items: Vec<ChargebackLineItem>,
    pub total_cost: f64,
}

// ─── Request / Query types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAllocationRequest {
    pub namespace: String,
    pub deployment: Option<String>,
    pub pod: Option<String>,
    pub container: Option<String>,
    pub labels: Option<HashMap<String, String>>,
    pub team: Option<String>,
    pub cost_center: Option<String>,
    pub usage: ResourceUsage,
    /// Requested (for efficiency calculation); if None, usage is used as both actual and requested.
    pub requested_usage: Option<ResourceUsage>,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateBudgetAlertRequest {
    pub name: String,
    pub namespace: Option<String>,
    pub team: Option<String>,
    pub cost_center: Option<String>,
    pub threshold_usd: f64,
    pub period: BudgetPeriod,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddCloudCostRequest {
    pub provider: CloudProvider,
    pub account_id: String,
    pub service: String,
    pub region: String,
    pub resource_id: Option<String>,
    pub tags: Option<HashMap<String, String>>,
    pub cost_usd: f64,
    pub usage_quantity: f64,
    pub usage_unit: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AllocationQuery {
    pub namespace: Option<String>,
    pub team: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
}
