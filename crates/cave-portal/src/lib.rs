//! Developer portal — compatible with Backstage.

pub mod catalog;
pub mod dashboard;
pub mod engine;
pub mod models;
pub mod plugins;
pub mod routes;
pub mod ui;
pub mod wraps;

/// Per-module admin views (etcd / cri / apiserver / iam / mesh / pg / vault)
/// + per-tenant dashboard. Pinned to backstage/backstage v1.50.3.
pub mod admin;

use axum::Router;
use cave_kernel::parity::ParityReport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PortalState {
    pub services: RwLock<Vec<models::Service>>,
    /// Parity reports collected from each module, keyed by module name.
    pub parity_cache: RwLock<HashMap<String, ParityReport>>,
    /// Catalog of every upstream UI consolidated into cave-portal — populated
    /// at startup with [`wraps::WrapRegistry::builtin`]. Mutable so deployment
    /// configuration can flag specific wraps as `Proxy`/`Planned` without a
    /// rebuild.
    pub wraps: RwLock<wraps::WrapRegistry>,
}

impl Default for PortalState {
    fn default() -> Self {
        Self {
            services: RwLock::new(Vec::new()),
            parity_cache: RwLock::new(HashMap::new()),
            wraps: RwLock::new(wraps::WrapRegistry::builtin()),
        }
    }
}

pub type State = PortalState;

pub fn router(state: Arc<PortalState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "portal";
