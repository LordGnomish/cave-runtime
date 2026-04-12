//! FinOps showback/chargeback — replaces Kubecost/CloudHealth
//!
//! Replaces: Kubecost, CloudHealth
//! Upstream tracking: see cave-upstream for monitored features.

pub mod allocator;
pub mod models;
pub mod reporting;
pub mod routes;

use std::sync::{Arc, Mutex};

use models::{BudgetPolicy, ChargebackRule, CostAllocation, CostCenter, Invoice, TagPolicy};

/// In-memory store (PostgreSQL integration via cave-db is future work).
pub struct CostAllocStore {
    pub cost_centers: Vec<CostCenter>,
    pub allocations: Vec<CostAllocation>,
    pub budget_policies: Vec<BudgetPolicy>,
    pub chargeback_rules: Vec<ChargebackRule>,
    pub tag_policies: Vec<TagPolicy>,
    pub invoices: Vec<Invoice>,
}

impl Default for CostAllocStore {
    fn default() -> Self {
        Self {
            cost_centers: Vec::new(),
            allocations: Vec::new(),
            budget_policies: Vec::new(),
            chargeback_rules: Vec::new(),
            tag_policies: Vec::new(),
            invoices: Vec::new(),
        }
    }
}

/// Module state — Arc<CostAllocState> is shared across all route handlers.
pub struct CostAllocState {
    pub store: Mutex<CostAllocStore>,
}

impl Default for CostAllocState {
    fn default() -> Self {
        Self {
            store: Mutex::new(CostAllocStore::default()),
        }
    }
}

/// Create the Axum router for this module.
pub fn router(state: Arc<CostAllocState>) -> axum::Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "cost-alloc";
