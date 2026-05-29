// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CAVE ERP — Sovereign open-source ERP.
//!
//! Compatible with: Odoo Community Edition.
//! Modules: HR, Recruitment, CRM, Sales, Purchase, Inventory, Accounting,
//! Manufacturing, Projects.

pub mod ar;
pub mod engine;
pub mod models;
pub mod modules;
pub mod payroll;
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
