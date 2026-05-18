// SPDX-License-Identifier: AGPL-3.0-or-later
//! CAVE CRM — Sovereign customer relationship management.
//!
//! Upstream: Twenty (twentyhq/twenty). Standalone, independent from
//! cave-erp's CRM submodule (which is being deprecated — see ADR-145).
//!
//! Function-based crate naming per ADR-147. Tenant isolation per
//! ADR-MULTI-TENANT-001 (Kamaji vCluster boundary; deep impl pending v0.2).

/// Re-exports the models module for public access.
pub mod models;

/// Re-exports the routes module for public access.
pub mod routes;

/// Re-exports the store module for public access.
pub mod store;

/// Re-exports the `CrmStore` type for convenient access.
pub use store::CrmStore;

/// Type alias for `CrmStore` to be used as application state.
pub type State = CrmStore;

use axum::Router;
use std::sync::Arc;

/// Creates the Axum router for the CRM module.
///
/// Takes an Arc-wrapped CrmStore and passes it to the routes module
/// to create the router instance.
pub fn router(state: Arc<CrmStore>) -> Router {
    routes::create_router(state)
}

/// Creates a new default CrmStore wrapped in an Arc.
///
/// Returns an Arc<CrmStore> initialized with default values.
pub fn new_state() -> Arc<CrmStore> {
    Arc::new(CrmStore::default())
}

/// The module name constant for CRM.
pub const MODULE_NAME: &str = "crm";

/// The upstream source repository identifier.
pub const UPSTREAM: &str = "twentyhq/twenty";
