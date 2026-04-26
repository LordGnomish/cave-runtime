//! Developer portal — compatible with Backstage.

pub mod catalog;
pub mod dashboard;
pub mod engine;
pub mod models;
pub mod routes;
pub mod ui;

use axum::Router;
use cave_kernel::parity::ParityReport;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PortalState {
    pub services: RwLock<Vec<models::Service>>,
    /// Parity reports collected from each module, keyed by module name.
    pub parity_cache: RwLock<HashMap<String, ParityReport>>,
}

impl Default for PortalState {
    fn default() -> Self {
        Self {
            services: RwLock::new(Vec::new()),
            parity_cache: RwLock::new(HashMap::new()),
        }
    }
}

pub type State = PortalState;

pub fn router(state: Arc<PortalState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "portal";
