//! FinOps & cost tracking — compatible with Kubecost/OpenCost
//!
//! Compatible with: OpenCost / Kubecost
//! Upstream tracking: see cave-upstream for monitored features.

pub mod allocation;
pub mod budget;
pub mod calculator;
pub mod models;
pub mod pricing;
pub mod recommendations;
pub mod reports;
pub mod routes;

use axum::Router;
use models::*;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

/// All in-memory state for the cost module.
pub struct CostStore {
    pub pricing_configs: HashMap<uuid::Uuid, PricingConfig>,
    pub cost_reports: HashMap<uuid::Uuid, CostReport>,
    pub budgets: HashMap<uuid::Uuid, Budget>,
    pub budget_alerts: Vec<BudgetAlert>,
    pub recommendations: HashMap<uuid::Uuid, CostRecommendation>,
    pub resource_costs: Vec<ResourceCost>,
}

impl Default for CostStore {
    fn default() -> Self {
        let mut pricing_configs = HashMap::new();
        let default_pricing = PricingConfig {
            id: uuid::Uuid::new_v4(),
            name: "default".to_string(),
            provider: CloudProvider::OnPrem,
            cpu_core_hour: 0.048,
            memory_gb_hour: 0.006,
            storage_gb_month: 0.10,
            network_egress_gb: 0.09,
            gpu_core_hour: 2.48,
            custom_rates: HashMap::new(),
            created_at: chrono::Utc::now(),
        };
        pricing_configs.insert(default_pricing.id, default_pricing);
        CostStore {
            pricing_configs,
            cost_reports: HashMap::new(),
            budgets: HashMap::new(),
            budget_alerts: Vec::new(),
            recommendations: HashMap::new(),
            resource_costs: Vec::new(),
        }
    }
}

pub struct CostState {
    pub store: Arc<RwLock<CostStore>>,
}

impl Default for CostState {
    fn default() -> Self {
        Self {
            store: Arc::new(RwLock::new(CostStore::default())),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<CostState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "cost";
