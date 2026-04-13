//! Enterprise-pluggable log aggregation backend trait.
//!
//! The CAVE Runtime ships with a built-in Loki-compatible log store as the
//! sovereign default. Enterprises can forward logs to their existing SIEM or
//! observability platform by implementing [`LogsBackend`].
//!
//! # Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────────────┐
//! │                       LogsBackend (trait)                            │
//! ├─────────────────────┬──────────────┬──────────────┬─────────────────┤
//! │  BuiltinLogsBackend │  Splunk      │  Datadog     │  CloudWatch     │
//! │  (Loki-compat       │  Adapter     │  Logs Adapter│  Logs Adapter   │
//! │   — sovereign)      │  (external)  │  (external)  │  (external)     │
//! └─────────────────────┴──────────────┴──────────────┴─────────────────┘
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::models::{Labels, LogEntry, TenantId};

// ─── Error type ────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum LogsBackendError {
    #[error("push failed: {0}")]
    PushFailed(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
    #[error("backend unreachable: {0}")]
    Unreachable(String),
    #[error("configuration error: {0}")]
    ConfigError(String),
}

pub type LogsResult<T> = Result<T, LogsBackendError>;

// ─── Log stream batch ──────────────────────────────────────────────────────

/// A batch of log entries sharing the same label set, ready to be pushed.
#[derive(Debug, Clone)]
pub struct LogStreamBatch {
    pub tenant_id: TenantId,
    pub labels: Labels,
    pub entries: Vec<LogEntry>,
}

// ─── LogsBackend trait ─────────────────────────────────────────────────────

/// Enterprise-pluggable log aggregation backend.
#[async_trait]
pub trait LogsBackend: Send + Sync + 'static {
    /// Push a batch of log streams to the backend.
    async fn push(&self, streams: Vec<LogStreamBatch>) -> LogsResult<()>;

    /// Execute a LogQL-compatible instant query.
    /// Backends that do not support LogQL may return an error.
    async fn query(
        &self,
        tenant_id: &str,
        logql: &str,
        limit: usize,
        start_ns: i64,
        end_ns: i64,
    ) -> LogsResult<serde_json::Value>;

    /// Return known label names for the tenant (for autocomplete).
    async fn label_names(&self, tenant_id: &str) -> LogsResult<Vec<String>>;

    /// Human-readable backend name.
    fn name(&self) -> &'static str;
}

// ─── Built-in implementation wrapper ──────────────────────────────────────

/// Wraps the sovereign LogStore as a `LogsBackend`.
pub struct BuiltinLogsBackend {
    pub store: std::sync::Arc<crate::store::LogStore>,
}

impl BuiltinLogsBackend {
    pub fn new(store: std::sync::Arc<crate::store::LogStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl LogsBackend for BuiltinLogsBackend {
    async fn push(&self, streams: Vec<LogStreamBatch>) -> LogsResult<()> {
        for batch in streams {
            self.store
                .push(&batch.tenant_id, batch.labels, batch.entries)
                .map_err(|e| LogsBackendError::PushFailed(e.to_string()))?;
        }
        Ok(())
    }

    async fn query(
        &self,
        tenant_id: &str,
        logql_expr: &str,
        limit: usize,
        start_ns: i64,
        end_ns: i64,
    ) -> LogsResult<serde_json::Value> {
        let query = crate::logql::parser::Parser::parse_query(logql_expr)
            .map_err(|e| LogsBackendError::QueryFailed(e.to_string()))?;
        let evaluator = crate::logql::Evaluator::new(self.store.clone());
        let result = match query {
            crate::logql::ast::Query::Log(ref log_q) => evaluator.eval_log_query(
                tenant_id,
                log_q,
                start_ns,
                end_ns,
                limit,
                crate::models::Direction::Forward,
            ),
            crate::logql::ast::Query::Metric(ref metric_q) => evaluator.eval_metric_query(
                tenant_id,
                metric_q,
                start_ns,
                end_ns,
                60_000_000_000, // 1m step in ns
            ),
        };
        serde_json::to_value(result)
            .map_err(|e| LogsBackendError::QueryFailed(e.to_string()))
    }

    async fn label_names(&self, tenant_id: &str) -> LogsResult<Vec<String>> {
        Ok(self.store.label_names(tenant_id))
    }

    fn name(&self) -> &'static str {
        "builtin-loki"
    }
}

// ─── Profile config ────────────────────────────────────────────────────────

/// Selects which logs backend the factory should instantiate.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogsBackendProfile {
    /// Built-in sovereign Loki-compatible log store (default).
    #[default]
    Builtin,
    /// Splunk Enterprise / Splunk Cloud via HTTP Event Collector (HEC).
    Splunk,
    /// Datadog Logs via HTTP API.
    Datadog,
    /// AWS CloudWatch Logs via PutLogEvents API.
    CloudWatch,
}
