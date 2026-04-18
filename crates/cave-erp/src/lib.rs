//! CAVE ERP — Sovereign open-source ERP.
//!
//! Replaces: Odoo Community Edition.
//! Modules: HR, Recruitment, CRM, Sales, Purchase, Inventory, Accounting,
//! Manufacturing, Projects.

pub mod engine;
pub mod models;
pub mod modules;
pub mod routes;
pub mod store;

pub use store::ErpStore;
pub type State = ErpStore;

use axum::Router;
use std::sync::Arc;

pub fn router(state: Arc<ErpStore>) -> Router {
    routes::create_router(state)
}

pub fn new_state() -> Arc<ErpStore> {
    Arc::new(ErpStore::default())
}

pub const MODULE_NAME: &str = "erp";
