<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/elastic-ellis
//! cave-trace — distributed tracing, Jaeger/Tempo replacement.
//!
//! Ingests spans, builds trace trees, detects anomalies, and exposes
//! service-dependency and latency analytics.

pub mod analyzer;
pub mod collector;
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
pub fn router(state: Arc<TraceState>) -> Router {
    routes::create_router(state)
}

<<<<<<< HEAD
=======
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
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/elastic-ellis
pub const MODULE_NAME: &str = "trace";
