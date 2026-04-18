//! On-call scheduling, paging, and escalation — replaces Grafana OnCall.
//!
//! Replaces: Grafana OnCall
//! Upstream tracking: see cave-upstream for monitored features.

use std::sync::Arc;
pub mod engine;
pub mod models;
pub mod routes;

pub use engine::OnCallError;
pub use routes::OnCallStore;

use axum::Router;

/// Create the axum router for this module.
pub fn router(state: Arc<OnCallStore>) -> Router {
    routes::create_router(state)
}

/// Convenience: build a fresh `OnCallStore` wrapped in an `Arc`.
pub fn new_state() -> Arc<OnCallStore> {
    Arc::new(OnCallStore::default())
}

pub const MODULE_NAME: &str = "oncall";

pub type State = OnCallStore;
