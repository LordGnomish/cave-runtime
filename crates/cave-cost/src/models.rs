use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CostEntry {
    pub id: Uuid,
    pub service: String,
    pub resource_id: String,
    pub team: String,
    pub environment: String,
    pub cost_usd: f64,
    pub date: NaiveDate,
    pub tags: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CostBudget {
    pub id: Uuid,
    pub name: String,
    pub team: String,
    pub monthly_limit_usd: f64,
    pub alert_threshold_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CostSummary {
    pub team: String,
    pub total_usd: f64,
    pub by_service: std::collections::HashMap<String, f64>,
}
