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
}
