use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    Aws,
    Gcp,
    Azure,
    OnPrem,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingConfig {
    pub id: Uuid,
    pub name: String,
    pub provider: CloudProvider,
    pub cpu_core_hour: f64,
    pub memory_gb_hour: f64,
    pub storage_gb_month: f64,
    pub network_egress_gb: f64,
    pub gpu_core_hour: f64,
    pub custom_rates: HashMap<String, f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCost {
    pub id: Uuid,
    pub resource_type: ResourceType,
    pub namespace: String,
    pub pod: Option<String>,
    pub controller: Option<String>,
    pub controller_kind: Option<String>,
    pub labels: HashMap<String, String>,
    pub annotations: HashMap<String, String>,
    pub cpu_cores: f64,
    pub cpu_cores_used: f64,
    pub memory_bytes: u64,
    pub memory_bytes_used: u64,
    pub storage_bytes: u64,
    pub network_egress_bytes: u64,
    pub gpu_cores: f64,
    pub cpu_cost: f64,
    pub memory_cost: f64,
    pub storage_cost: f64,
    pub network_cost: f64,
    pub gpu_cost: f64,
    pub total_cost: f64,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceType {
    Pod,
    Namespace,
    Controller,
    Label,
    Node,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAllocation {
    pub namespace: String,
    pub labels: HashMap<String, String>,
    pub controller: Option<String>,
    pub total_cost: f64,
    pub cpu_cost: f64,
    pub memory_cost: f64,
    pub storage_cost: f64,
    pub network_cost: f64,
    pub idle_cost: f64,
    pub shared_cost: f64,
    /// 0.0–1.0
    pub efficiency: f64,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportWindow {
    LastDay,
    LastWeek,
    LastMonth,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostReport {
    pub id: Uuid,
    pub name: String,
    pub window: ReportWindow,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub aggregate_by: AggregateBy,
    pub total_cost: f64,
    pub allocations: Vec<CostAllocation>,
    pub idle_cost: f64,
    pub system_cost: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateBy {
    Namespace,
    Label,
    Annotation,
    Controller,
    Node,
    Pod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Budget {
    pub id: Uuid,
    pub name: String,
    pub namespace: Option<String>,
    pub label_selector: HashMap<String, String>,
    pub monthly_limit_usd: f64,
    /// e.g. 80.0 for 80%
    pub alert_threshold_percent: f64,
    /// alert if trending over X% above limit
    pub alert_trend_percent: Option<f64>,
    pub current_spend: f64,
    pub forecasted_spend: f64,
    pub status: BudgetStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    Ok,
    Warning,
    Exceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAlert {
    pub budget_id: Uuid,
    pub budget_name: String,
    pub alert_type: AlertType,
    pub current_spend: f64,
    pub limit: f64,
    pub percent_used: f64,
    pub triggered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertType {
    ThresholdExceeded,
    TrendBased,
    ForecastExceeded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecommendation {
    pub id: Uuid,
    pub kind: RecommendationKind,
    pub namespace: String,
    pub resource_name: String,
    pub current_cpu_request: Option<f64>,
    pub recommended_cpu_request: Option<f64>,
    pub current_memory_request: Option<u64>,
    pub recommended_memory_request: Option<u64>,
    pub current_monthly_cost: f64,
    pub recommended_monthly_cost: f64,
    pub estimated_savings: f64,
    /// 0.0–1.0
    pub confidence: f64,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationKind {
    Rightsizing,
    OrphanedResource,
    LowUtilization,
    SpotInstance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostTrend {
    pub namespace: Option<String>,
    pub data_points: Vec<TrendPoint>,
    pub forecast_points: Vec<TrendPoint>,
    /// percentage
    pub month_over_month_change: f64,
    pub projected_monthly_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendPoint {
    pub timestamp: DateTime<Utc>,
    pub cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowbackReport {
    pub id: Uuid,
    pub name: String,
    pub report_type: ShowbackType,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub line_items: Vec<ShowbackLineItem>,
    pub total_cost: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowbackType {
    Showback,
    Chargeback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowbackLineItem {
    pub team: String,
    pub namespace: String,
    pub cost: f64,
    pub cpu_cost: f64,
    pub memory_cost: f64,
    pub storage_cost: f64,
}
