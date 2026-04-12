//! CAVE GitOps Config — Platform-as-a-product promise management.
//!
//! Replaces: Kratix
//! Promise-based platform API with pipeline transformation and GitOps state management.

pub mod engine;
pub mod models;
pub mod routes;
pub mod store;

use axum::Router;
use routes::GitOpsAppState;
use std::sync::Arc;

pub struct GitOpsConfigState {
    pub app: Arc<GitOpsAppState>,
}

impl Default for GitOpsConfigState {
    fn default() -> Self {
        Self {
            app: Arc::new(GitOpsAppState::default()),
//! Platform API — Kratix/Crossplane Compositions replacement for CAVE.
//! Platform engineers define "Promises" (self-service capabilities).
//! Application teams request capabilities; this crate orchestrates
//! provisioning across cave-pg, cave-vault, cave-dns, cave-metrics, etc.
//! Replaces: Kratix, Crossplane Compositions
//! Upstream tracking: see cave-upstream for monitored features.
pub mod composition;
pub mod promise;
use models::{ComplianceCheck, Composition, Environment, PromiseRequest, ResourceClaim};
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
impl Default for AppState {
            promises: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            compositions: Mutex::new(Vec::new()),
            environments: Mutex::new(Vec::new()),
            claims: Mutex::new(Vec::new()),
            compliance_checks: Mutex::new(Vec::new()),
        }
    }
}

pub fn router(state: Arc<GitOpsAppState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "gitops-config";
// ---------------------------------------------------------------------------
// Router factory
// ---------------------------------------------------------------------------
/// Create the Axum router for the gitops-config module.
pub fn router(state: Arc<AppState>) -> Router {
