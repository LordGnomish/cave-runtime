//! PII detection and redaction — replaces Presidio.
//!
//! Replaces: Presidio
//! Upstream tracking: see cave-upstream for monitored features.

pub mod engine;
pub mod models;
pub mod routes;

pub use engine::{DefaultPiiEngine, PiiEngine};
pub use routes::PiiStore;

use axum::Router;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Create the axum router for this module.
pub fn router(state: Arc<RwLock<PiiStore>>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "pii";
