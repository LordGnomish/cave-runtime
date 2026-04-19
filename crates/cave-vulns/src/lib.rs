//! Vulnerability aggregation hub — compatible with DefectDojo
//!
//! Compatible with: DefectDojo
//! Upstream tracking: see cave-upstream for monitored features.

pub mod engine;
pub mod models;
pub mod routes;

use axum::Router;
use cave_db::Storage;
use std::sync::Arc;

/// Module state.
pub struct State {
    pub storage: Arc<dyn Storage>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            storage: Arc::new(cave_db::MemoryStorage::default()),
        }
    }
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "vulns";
