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
