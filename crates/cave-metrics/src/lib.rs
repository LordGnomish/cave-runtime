//! CAVE Metrics — time-series metrics ingestion and query engine.
//!
//! Replaces Prometheus + Thanos with a Rust-native implementation.
//! Supports remote_write ingestion, PromQL-compatible query API,
//! series metadata, and Prometheus exposition format for self-metrics.
//!
//! ## Upstream Compatibility: Prometheus
//! - Remote Write: POST /api/v1/write (Prometheus remote_write protobuf)
//! - Query API:    GET  /api/v1/query, /api/v1/query_range
//! - Metadata:     GET  /api/v1/series, /api/v1/labels, /api/v1/label/:name/values
//! - Self-metrics: GET  /metrics (Prometheus exposition format)
//! - Response envelope: {"status":"success","data":{"resultType":"...","result":[...]}}
//!
//! ## Upstream Tracking: Prometheus
//! - GitHub: https://github.com/prometheus/prometheus
//! - Tracked: remote_write protocol v1/v2, HTTP API spec, exposition format

pub mod models;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state shared across request handlers.
pub struct MetricsState {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for the metrics module.
pub fn router(state: Arc<MetricsState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "metrics";
