//! Platform API — Kratix/Crossplane Compositions replacement for CAVE.
//!
//! Platform engineers define "Promises" (self-service capabilities).
//! Application teams request capabilities; this crate orchestrates
//! provisioning across cave-pg, cave-vault, cave-dns, cave-metrics, etc.
//!
//! Replaces: Kratix, Crossplane Compositions
//! Upstream tracking: see cave-upstream for monitored features.

pub mod composition;
pub mod models;
pub mod promise;
pub mod routes;

use axum::Router;
use models::{ComplianceCheck, Composition, Environment, PromiseRequest, ResourceClaim};
use std::sync::Arc;
use tokio::sync::Mutex;

pub const MODULE_NAME: &str = "gitops-config";

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// All in-memory state for the gitops-config module.
///
/// In production each collection would be backed by a PostgreSQL table via
/// `cave-db`; the `Arc<Mutex<Vec<_>>>` pattern matches the rest of the
/// workspace and keeps the module compilable without a live database.
pub struct AppState {
    pub promises: Mutex<Vec<models::Promise>>,
    pub requests: Mutex<Vec<PromiseRequest>>,
    pub compositions: Mutex<Vec<Composition>>,
    pub environments: Mutex<Vec<Environment>>,
    pub claims: Mutex<Vec<ResourceClaim>>,
    pub compliance_checks: Mutex<Vec<ComplianceCheck>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            promises: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            compositions: Mutex::new(Vec::new()),
            environments: Mutex::new(Vec::new()),
            claims: Mutex::new(Vec::new()),
            compliance_checks: Mutex::new(Vec::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Router factory
// ---------------------------------------------------------------------------

/// Create the Axum router for the gitops-config module.
pub fn router(state: Arc<AppState>) -> Router {
    routes::create_router(state)
}
