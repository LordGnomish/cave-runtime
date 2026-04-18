//! Developer portal — replaces Backstage.

pub mod dashboard;
pub mod engine;
pub mod models;
pub mod routes;
pub mod ui;

use axum::Router;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct PortalState {
    pub services: RwLock<Vec<models::Service>>,
}

impl Default for PortalState {
    fn default() -> Self {
        Self { services: RwLock::new(Vec::new()) }
    }
}

pub type State = PortalState;

pub fn router(state: Arc<PortalState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "portal";
