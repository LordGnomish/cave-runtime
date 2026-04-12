use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Core enums ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanStatus {
    Ok,
    Error,
    Unset,
}

impl Default for SpanStatus {
    fn default() -> Self {
        SpanStatus::Unset
    }
}

// ── Span primitives ───────────────────────────────────────────────────────────

/// A timestamped log record attached to a span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanEvent {
    pub timestamp: DateTime<Utc>,
    pub name: String,
    pub attributes: serde_json::Value,
}

/// A causal link to a span in another (or the same) trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanLink {
    pub trace_id: Uuid,
    pub span_id: Uuid,
    pub attributes: serde_json::Value,
}

/// A single unit of work in a distributed trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub trace_id: Uuid,
    pub span_id: Uuid,
    pub parent_span_id: Option<Uuid>,
    pub operation: String,
    pub service: String,
    pub start_time: DateTime<Utc>,
    pub duration_ms: f64,
    pub status: SpanStatus,
    pub tags: HashMap<String, serde_json::Value>,
    pub events: Vec<SpanEvent>,
    pub links: Vec<SpanLink>,
}

// ── Tree representation ───────────────────────────────────────────────────────

/// A span and its children, forming part of a trace tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceNode {
    pub span: Span,
    pub children: Vec<TraceNode>,
}

// ── Aggregate trace view ──────────────────────────────────────────────────────

/// A complete trace assembled from a set of spans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    pub trace_id: Uuid,
    pub root_spans: Vec<Span>,
    pub all_spans: Vec<Span>,
    pub service_count: usize,
    pub span_count: usize,
    pub total_duration_ms: f64,
    pub start_time: Option<DateTime<Utc>>,
    pub status: SpanStatus,
}

// ── Query / search ────────────────────────────────────────────────────────────

/// Query parameters for `GET /api/v1/traces/search`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceQuery {
    pub service: Option<String>,
    pub operation: Option<String>,
    pub min_duration_ms: Option<f64>,
    pub max_duration_ms: Option<f64>,
    pub status: Option<SpanStatus>,
    pub limit: Option<usize>,
}

// ── Service map ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceNode {
    pub name: String,
    pub span_count: u64,
    pub error_count: u64,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDependency {
    pub source: String,
    pub target: String,
    pub call_count: u64,
    pub error_count: u64,
    pub avg_duration_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceMap {
    pub services: Vec<ServiceNode>,
    pub dependencies: Vec<ServiceDependency>,
}

// ── Statistics ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStats {
    pub service: String,
    pub operation: Option<String>,
    pub total_spans: u64,
    pub error_rate: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub avg_ms: f64,
}

// ── Anomaly detection ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnomalousSpan {
    pub span: Span,
    pub reason: String,
    /// Z-score (or ≥3.0 for error spans regardless of latency).
    pub severity: f64,
}

// ── Error propagation ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPropagation {
    pub trace_id: Uuid,
    pub error_chain: Vec<Uuid>,
    pub root_cause_service: String,
    pub affected_services: Vec<String>,
    pub total_error_spans: usize,
}

// ── Bottleneck ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    pub service: String,
    pub operation: String,
    pub avg_duration_ms: f64,
    pub call_count: u64,
    pub total_time_ms: f64,
    /// Percentage of the total sampled time consumed by this service+operation.
    pub percentage_of_trace: f64,
}

// ── DTOs ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct IngestSpanRequest {
    pub trace_id: Option<Uuid>,
    pub span_id: Option<Uuid>,
    pub parent_span_id: Option<Uuid>,
    pub operation: String,
    pub service: String,
    pub start_time: Option<DateTime<Utc>>,
    pub duration_ms: f64,
    pub status: Option<SpanStatus>,
    pub tags: Option<HashMap<String, serde_json::Value>>,
    pub events: Option<Vec<SpanEvent>>,
    pub links: Option<Vec<SpanLink>>,
}

#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub trace_id: Uuid,
    pub span_id: Uuid,
    pub ingested: bool,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub traces: Vec<Trace>,
    pub total: usize,
}
