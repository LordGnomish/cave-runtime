//! CAVE Trace — distributed tracing backend.
//!
//! Replaces Jaeger / Grafana Tempo with a Rust-native implementation.
//! Accepts traces via the OpenTelemetry Protocol (OTLP) over HTTP.
//!
//! ## Upstream Compatibility: OpenTelemetry (OTLP)
//! - Trace receiver: POST /v1/traces
//!   Accepts application/x-protobuf (binary protobuf) OR
//!          application/json  (OTLP JSON encoding)
//!   Response: ExportTraceServiceResponse (empty JSON `{}` on success)
//!
//! ## Upstream Tracking: OpenTelemetry Collector
//! - GitHub: https://github.com/open-telemetry/opentelemetry-collector
//! - Spec:   https://opentelemetry.io/docs/specs/otlp/
//! - Tracked: OTLP/HTTP receiver protocol

pub mod models;
pub mod routes;

use axum::Router;
use cave_db::CavePool;
use std::sync::Arc;

/// Module state shared across request handlers.
pub struct TraceState {
    pub pool: Arc<CavePool>,
}

/// Create the axum router for the trace module.
pub fn router(state: Arc<TraceState>) -> Router {
    routes::create_router(state)
}

pub const MODULE_NAME: &str = "trace";
