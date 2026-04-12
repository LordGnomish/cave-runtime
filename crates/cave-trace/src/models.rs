<<<<<<< HEAD
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
=======
//! OTLP-compatible data models for trace ingestion.
//!
//! These types represent the JSON encoding of the OTLP
//! ExportTraceServiceRequest / ExportTraceServiceResponse as specified in:
//! https://opentelemetry.io/docs/specs/otlp/#otlphttp-request
//!
//! Binary protobuf encoding is also accepted on the same endpoint;
//! JSON models are used for the structured-logging path.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// OTLP ExportTraceServiceRequest (JSON encoding)
// ---------------------------------------------------------------------------

/// Top-level OTLP trace export request.
/// POST /v1/traces with Content-Type: application/json
#[derive(Debug, Deserialize)]
pub struct ExportTraceServiceRequest {
    #[serde(rename = "resourceSpans", default)]
    pub resource_spans: Vec<ResourceSpans>,
}

/// Spans grouped by resource (service).
#[derive(Debug, Deserialize)]
pub struct ResourceSpans {
    pub resource: Option<Resource>,
    #[serde(rename = "scopeSpans", default)]
    pub scope_spans: Vec<ScopeSpans>,
    /// Deployment schema URL (optional)
    #[serde(rename = "schemaUrl", default)]
    pub schema_url: String,
}

/// The resource (service) that produced the spans.
#[derive(Debug, Deserialize)]
pub struct Resource {
    /// Key-value attributes, e.g. service.name, host.name
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

/// Spans grouped by instrumentation scope (library).
#[derive(Debug, Deserialize)]
pub struct ScopeSpans {
    pub scope: Option<InstrumentationScope>,
    #[serde(default)]
    pub spans: Vec<Span>,
    #[serde(rename = "schemaUrl", default)]
    pub schema_url: String,
}

/// Instrumentation scope (library/component that created the spans).
#[derive(Debug, Deserialize)]
pub struct InstrumentationScope {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

/// A single trace span.
#[derive(Debug, Deserialize)]
pub struct Span {
    /// 16-byte trace ID as hex string
    #[serde(rename = "traceId", default)]
    pub trace_id: String,
    /// 8-byte span ID as hex string
    #[serde(rename = "spanId", default)]
    pub span_id: String,
    /// Parent span ID (empty for root spans)
    #[serde(rename = "parentSpanId", default)]
    pub parent_span_id: String,
    #[serde(default)]
    pub name: String,
    /// SpanKind: 0=UNSPECIFIED, 1=INTERNAL, 2=SERVER, 3=CLIENT, 4=PRODUCER, 5=CONSUMER
    #[serde(default)]
    pub kind: i32,
    /// Start time in Unix nanoseconds (string in JSON encoding)
    #[serde(rename = "startTimeUnixNano", default)]
    pub start_time_unix_nano: String,
    /// End time in Unix nanoseconds (string in JSON encoding)
    #[serde(rename = "endTimeUnixNano", default)]
    pub end_time_unix_nano: String,
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
    #[serde(default)]
    pub events: Vec<SpanEvent>,
    #[serde(default)]
    pub links: Vec<SpanLink>,
    pub status: Option<SpanStatus>,
}

/// A key-value attribute pair.
#[derive(Debug, Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: AnyValue,
}

/// OTLP AnyValue — union of supported value types.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AnyValue {
    String { #[serde(rename = "stringValue")] string_value: String },
    Bool   { #[serde(rename = "boolValue")]   bool_value:   bool },
    Int    { #[serde(rename = "intValue")]     int_value:    i64 },
    Double { #[serde(rename = "doubleValue")]  double_value: f64 },
    // Arrays / kvlist omitted for brevity; extend when needed
}

/// An event within a span (formerly called a "log annotation").
#[derive(Debug, Deserialize)]
pub struct SpanEvent {
    #[serde(rename = "timeUnixNano", default)]
    pub time_unix_nano: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

/// A causal link to another span.
#[derive(Debug, Deserialize)]
pub struct SpanLink {
    #[serde(rename = "traceId", default)]
    pub trace_id: String,
    #[serde(rename = "spanId", default)]
    pub span_id: String,
    #[serde(default)]
    pub attributes: Vec<KeyValue>,
}

/// Span status.
#[derive(Debug, Deserialize)]
pub struct SpanStatus {
    #[serde(default)]
    pub message: String,
    /// StatusCode: 0=UNSET, 1=OK, 2=ERROR
    pub code: Option<i32>,
}

// ---------------------------------------------------------------------------
// OTLP ExportTraceServiceResponse
// ---------------------------------------------------------------------------

/// Successful response — an empty object per OTLP spec.
#[derive(Debug, Serialize)]
pub struct ExportTraceServiceResponse {
    /// Partial success info (omitted when all spans were accepted).
    #[serde(rename = "partialSuccess", skip_serializing_if = "Option::is_none")]
    pub partial_success: Option<PartialSuccess>,
}

#[derive(Debug, Serialize)]
pub struct PartialSuccess {
    #[serde(rename = "rejectedSpans")]
    pub rejected_spans: i64,
    #[serde(rename = "errorMessage")]
    pub error_message: String,
>>>>>>> claude/gallant-cartwright
}
