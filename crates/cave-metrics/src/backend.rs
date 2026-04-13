//! Enterprise-pluggable metrics backend trait.
//!
//! The CAVE Runtime ships with a built-in Prometheus/VictoriaMetrics-compatible
//! TSDB as the sovereign default. Enterprises can route metrics to their
//! existing observability platform by implementing [`MetricsBackend`].
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────┐
//! │                     MetricsBackend (trait)                           │
//! ├─────────────────────┬──────────────┬──────────────┬─────────────────┤
//! │  BuiltinMetrics     │  Datadog     │  New Relic   │  CloudWatch     │
//! │  (TSDB + PromQL     │  Adapter     │  Adapter     │  Adapter        │
//! │   — sovereign)      │  (external)  │  (external)  │  (external)     │
//! └─────────────────────┴──────────────┴──────────────┴─────────────────┘
//!         ▲
//!   selected by MetricsBackendProfile::from_config(...)
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{Labels, Sample, TimeSeries, QueryResult};

// ─── QueryResult → serde_json::Value ──────────────────────────────────────

/// Convert a `QueryResult` to a `serde_json::Value` for API responses.
/// `QueryResult` doesn't implement `Serialize` directly, so we map manually.
fn query_result_to_value(r: QueryResult) -> serde_json::Value {
    match r {
        QueryResult::Scalar(v) => serde_json::json!({ "resultType": "scalar", "result": v }),
        QueryResult::String(s) => serde_json::json!({ "resultType": "string", "result": s }),
        QueryResult::InstantVector(iv) => {
            let result: Vec<serde_json::Value> = iv
                .into_iter()
                .map(|(labels, v)| serde_json::json!({ "metric": labels, "value": v }))
                .collect();
            serde_json::json!({ "resultType": "vector", "result": result })
        }
        QueryResult::RangeVector(rv) => {
            let result: Vec<serde_json::Value> = rv
                .into_iter()
                .map(|(labels, samples)| {
                    let values: Vec<serde_json::Value> = samples
                        .iter()
                        .map(|s| serde_json::json!([s.timestamp_ms, s.value]))
                        .collect();
                    serde_json::json!({ "metric": labels, "values": values })
                })
                .collect();
            serde_json::json!({ "resultType": "matrix", "result": result })
        }
    }
}

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MetricsBackendError {
    #[error("write failed: {0}")]
    WriteFailed(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
    #[error("backend unreachable: {0}")]
    Unreachable(String),
    #[error("configuration error: {0}")]
    ConfigError(String),
}

pub type MetricsResult<T> = Result<T, MetricsBackendError>;

// ─── MetricsBackend trait ──────────────────────────────────────────────────

/// Enterprise-pluggable metrics backend.
///
/// All internal CAVE components write metrics through this trait. The factory
/// selects either the built-in TSDB or an external adapter at startup.
#[async_trait]
pub trait MetricsBackend: Send + Sync + 'static {
    /// Write a batch of time series samples to the backend.
    async fn write(&self, batch: Vec<TimeSeries>) -> MetricsResult<()>;

    /// Write a single labeled sample (convenience wrapper).
    async fn record(&self, labels: Labels, sample: Sample) -> MetricsResult<()> {
        let mut ts = TimeSeries::new(labels);
        ts.push(sample);
        self.write(vec![ts]).await
    }

    /// Execute an instant PromQL-compatible query at `timestamp_ms`.
    /// Returns serialisable JSON suitable for Prometheus API responses.
    /// Backends that do not support PromQL should return an error.
    async fn query_instant(
        &self,
        expr: &str,
        timestamp_ms: i64,
    ) -> MetricsResult<serde_json::Value>;

    /// Execute a range PromQL-compatible query.
    async fn query_range(
        &self,
        expr: &str,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> MetricsResult<serde_json::Value>;

    /// Return currently known label names (for autocomplete / federation).
    async fn label_names(&self) -> MetricsResult<Vec<String>>;

    /// Human-readable backend name — used in logs and `/ready` output.
    fn name(&self) -> &'static str;
}

// ─── Built-in implementation wrapper ──────────────────────────────────────

/// Wraps the sovereign TSDB + PromQL engine as a `MetricsBackend`.
pub struct BuiltinMetricsBackend {
    pub state: std::sync::Arc<crate::state::MetricsState>,
}

impl BuiltinMetricsBackend {
    pub fn new(state: std::sync::Arc<crate::state::MetricsState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl MetricsBackend for BuiltinMetricsBackend {
    async fn write(&self, batch: Vec<TimeSeries>) -> MetricsResult<()> {
        for ts in batch {
            for sample in ts.samples {
                self.state.tsdb.append(ts.labels.clone(), sample);
            }
        }
        Ok(())
    }

    async fn query_instant(
        &self,
        expr: &str,
        timestamp_ms: i64,
    ) -> MetricsResult<serde_json::Value> {
        let ast = crate::promql::parse(expr)
            .map_err(|e| MetricsBackendError::QueryFailed(e.to_string()))?;
        let result = self
            .state
            .engine
            .eval_instant(&ast, timestamp_ms)
            .map_err(|e| MetricsBackendError::QueryFailed(e.to_string()))?;
        Ok(query_result_to_value(result))
    }

    async fn query_range(
        &self,
        expr: &str,
        start_ms: i64,
        end_ms: i64,
        step_ms: i64,
    ) -> MetricsResult<serde_json::Value> {
        let ast = crate::promql::parse(expr)
            .map_err(|e| MetricsBackendError::QueryFailed(e.to_string()))?;
        let result = self
            .state
            .engine
            .eval_range(&ast, start_ms, end_ms, step_ms)
            .map_err(|e| MetricsBackendError::QueryFailed(e.to_string()))?;
        let steps: Vec<serde_json::Value> = result
            .into_iter()
            .map(|(ts_ms, r)| serde_json::json!({ "timestamp_ms": ts_ms, "result": query_result_to_value(r) }))
            .collect();
        Ok(serde_json::json!({ "resultType": "range", "steps": steps }))
    }

    async fn label_names(&self) -> MetricsResult<Vec<String>> {
        Ok(self.state.tsdb.label_names(&[]))
    }

    fn name(&self) -> &'static str {
        "builtin-tsdb"
    }
}

// ─── Profile config ────────────────────────────────────────────────────────

/// Selects which metrics backend the factory should instantiate.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MetricsBackendProfile {
    /// Built-in sovereign TSDB + PromQL engine (default).
    #[default]
    Builtin,
    /// Datadog Metrics API (DogStatsD / HTTP).
    Datadog,
    /// New Relic Metric API.
    NewRelic,
    /// AWS CloudWatch Metrics.
    CloudWatch,
}
