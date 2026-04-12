//! Alert routing & management — replaces Alertmanager
//!
//! Replaces: Alertmanager
//! Upstream tracking: see cave-upstream for monitored features.

pub mod models;
pub mod engine;
pub mod routes;

use axum::Router;
use cave_db::Storage;
use std::sync::Arc;

/// Module state.
pub struct State {
    pub storage: Arc<dyn Storage>,
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "alerts";
