//! Developer portal — replaces Backstage
//!
//! Single-page web application that serves as the unified UI for all 30 CAVE
//! runtime modules. Routes are served at / (SPA) and /api/v1/portal/* (JSON API).
//!
//! Replaces: Backstage
//! Upstream tracking: see cave-upstream for monitored features.

pub mod engine;
pub mod dashboard;
pub mod engine;
pub mod models;
pub mod routes;
pub mod ui;

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

pub const MODULE_NAME: &str = "portal";
