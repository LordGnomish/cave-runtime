//! CAVE CRM — Sovereign customer relationship management.
//!
//! Upstream: Twenty (twentyhq/twenty). Standalone, independent from
//! cave-erp's CRM submodule (which is being deprecated — see ADR-145).
//!
//! Function-based crate naming per ADR-147. Tenant isolation per
//! ADR-MULTI-TENANT-001 (Kamaji vCluster boundary; deep impl pending v0.2).

pub mod models;
pub mod routes;
pub mod store;

pub use store::CrmStore;
pub type State = CrmStore;

use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<CrmStore>) -> Router {
    routes::create_router(state)
}

pub fn new_state() -> Arc<CrmStore> {
    Arc::new(CrmStore::default())
}

pub const MODULE_NAME: &str = "crm";
pub const UPSTREAM: &str = "twentyhq/twenty";
