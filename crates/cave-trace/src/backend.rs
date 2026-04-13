//! Enterprise-pluggable distributed tracing backend trait.
//!
//! The CAVE Runtime ships with a built-in Tempo/Jaeger-compatible tracing
//! engine as the sovereign default. Enterprises can forward traces to their
//! existing APM platform by implementing [`TraceBackend`].
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────┐
//! │                      TraceBackend (trait)                            │
//! ├─────────────────────┬──────────────┬──────────────┬─────────────────┤
//! │  BuiltinTraceBackend│  Datadog     │  Jaeger      │  New Relic      │
//! │  (Tempo/Jaeger-compat│  APM Adapter │  Remote Adpt │  Adapter        │
//! │   — sovereign)      │  (external)  │  (external)  │  (external)     │
//! └─────────────────────┴──────────────┴──────────────┴─────────────────┘
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{Span, TraceId};

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum TraceBackendError {
    #[error("ingest failed: {0}")]
    IngestFailed(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
    #[error("trace not found: {0}")]
    NotFound(String),
    #[error("backend unreachable: {0}")]
    Unreachable(String),
    #[error("configuration error: {0}")]
    ConfigError(String),
}

pub type TraceResult<T> = Result<T, TraceBackendError>;

// ─── TraceBackend trait ────────────────────────────────────────────────────

/// Enterprise-pluggable distributed tracing backend.
///
/// All CAVE instrumentation writes spans through this trait. The factory
/// selects either the built-in Tempo/Jaeger engine or an external adapter.
#[async_trait]
pub trait TraceBackend: Send + Sync + 'static {
    /// Ingest a batch of spans. Called on every trace write path.
    async fn ingest(&self, spans: Vec<Span>) -> TraceResult<()>;

    /// Retrieve a complete trace by ID.
    async fn get_trace(&self, trace_id: TraceId) -> TraceResult<Vec<Span>>;

    /// Search for traces matching the given criteria (serialized as JSON params).
    async fn search(
        &self,
        service: Option<&str>,
        operation: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> TraceResult<Vec<TraceId>>;

    /// Return service names currently in the store (for UI autocomplete).
    async fn services(&self) -> TraceResult<Vec<String>>;

    /// Human-readable backend name.
    fn name(&self) -> &'static str;
}

// ─── Built-in implementation wrapper ──────────────────────────────────────

/// Wraps the sovereign TraceStore as a `TraceBackend`.
pub struct BuiltinTraceBackend {
    pub state: std::sync::Arc<crate::TraceState>,
}

impl BuiltinTraceBackend {
    pub fn new(state: std::sync::Arc<crate::TraceState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl TraceBackend for BuiltinTraceBackend {
    async fn ingest(&self, spans: Vec<Span>) -> TraceResult<()> {
        // Apply sampling decision then write to store.
        let keep = spans.iter().any(|s| {
            self.state.sampler.should_sample(s.trace_id, s).is_sample()
        });
        if keep {
            self.state.spm_registry.record_spans(&spans);
            self.state.store.write().await.ingest_spans(spans);
        }
        Ok(())
    }

    async fn get_trace(&self, trace_id: TraceId) -> TraceResult<Vec<Span>> {
        self.state
            .query
            .get_trace_spans(trace_id)
            .await
            .map_err(|e| TraceBackendError::NotFound(e.to_string()))
    }

    async fn search(
        &self,
        service: Option<&str>,
        operation: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> TraceResult<Vec<TraceId>> {
        let query = crate::types::TraceSearchQuery {
            service: service.map(|s| s.to_string()),
            operation: operation.map(|s| s.to_string()),
            start_time_ns: Some(start_ms as u64 * 1_000_000),
            end_time_ns: Some(end_ms as u64 * 1_000_000),
            limit: Some(limit),
            ..Default::default()
        };
        let traces = self
            .state
            .query
            .search(&query)
            .await
            .map_err(|e| TraceBackendError::QueryFailed(e.to_string()))?;
        Ok(traces.into_iter().map(|t| t.trace_id).collect())
    }

    async fn services(&self) -> TraceResult<Vec<String>> {
        Ok(self.state.query.list_services(None).await)
    }

    fn name(&self) -> &'static str {
        "builtin-tempo"
    }
}

// ─── Profile config ────────────────────────────────────────────────────────

/// Selects which trace backend the factory should instantiate.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TraceBackendProfile {
    /// Built-in sovereign Tempo/Jaeger-compatible tracing engine (default).
    #[default]
    Builtin,
    /// Datadog APM via Datadog Agent or HTTP API.
    Datadog,
    /// External Jaeger collector via HTTP/gRPC.
    Jaeger,
    /// New Relic Traces via OpenTelemetry endpoint.
    NewRelic,
}
