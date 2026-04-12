//! CAVE Logs — structured log ingestion and query engine.
//!
//! Replaces Loki with a Rust-native implementation.
//! Supports Loki push API, LogQL instant and range queries,
//! and label enumeration for Grafana/Alloy compatibility.
//!
//! ## Upstream Compatibility: Loki
//! - Push:          POST /loki/api/v1/push
//! - Instant query: GET  /loki/api/v1/query
//! - Range query:   GET  /loki/api/v1/query_range
//! - Labels:        GET  /loki/api/v1/labels
//! - Label values:  GET  /loki/api/v1/label/:name/values
//!
//! ## Upstream Tracking: Grafana Loki
//! - GitHub: https://github.com/grafana/loki
//! - Tracked: push API, LogQL query API, label API

pub mod models;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state shared across request handlers.
pub struct LogsState {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for the logs module.
pub fn router(state: Arc<LogsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "logs";
