//! Continuous profiling — replaces Pyroscope/Parca
//!
//! Replaces: Pyroscope
//! Upstream tracking: see cave-upstream for monitored features.

pub mod engine;
pub mod models;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state.
pub struct State {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for this module.
pub fn router(state: Arc<State>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "profiler";
