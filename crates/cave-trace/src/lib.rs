//! cave-trace — distributed tracing, Jaeger/Tempo replacement.
//!
//! Ingests spans, builds trace trees, detects anomalies, and exposes
//! service-dependency and latency analytics.

pub mod analyzer;
pub mod collector;
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
use std::sync::Arc;
use tokio::sync::Mutex;

/// In-memory span store.
pub struct TraceStore {
    pub spans: Vec<models::Span>,
}

/// Shared module state — cheap to clone (Arc inside).
pub struct TraceState {
    pub store: Arc<Mutex<TraceStore>>,
}

impl TraceState {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(TraceStore { spans: Vec::new() })),
        }
    }
}

impl Default for TraceState {
    fn default() -> Self {
        Self::new()
    }
}

/// Return the axum sub-router for all tracing endpoints.
use cave_db::CavePool;
/// Module state shared across request handlers.
    pub pool: Arc<CavePool>,
/// Create the axum router for the trace module.
pub fn router(state: Arc<TraceState>) -> Router {
    routes::create_router(state)
}

//! CAVE Trace — Jaeger replacement.
pub mod error;
pub mod types;
pub mod storage;
pub mod query;
pub mod otlp;
pub mod dependency;
pub mod sampling;
pub mod comparison;
pub mod routes;
pub use storage::TraceStore;
pub use error::{TraceError, TraceResult};
pub const MODULE_NAME: &str = "trace";
