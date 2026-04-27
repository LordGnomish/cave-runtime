//! cave-tracing — Sovereign OpenTelemetry-compatible tracing SDK.
//!
//! Companion to cave-trace (the storage/query backend). Where cave-trace
//! ingests and serves spans, cave-tracing is what application code links
//! against to *produce* spans.
//!
//! Shape:
//!
//! ```text
//! TracerProvider ─┬─ Tracer ─ SpanBuilder ─ Span
//!                 ├─ Sampler  (AlwaysOn / AlwaysOff / TraceIdRatioBased / ParentBased)
//!                 ├─ SpanProcessor (BatchSpanProcessor / InMemoryProcessor)
//!                 └─ Resource + tenant
//!
//! BatchSpanProcessor ─ async ticker ─ SpanExporter
//!                                      ├─ NoopExporter
//!                                      ├─ InMemoryExporter (tests)
//!                                      ├─ OtlpHttpExporter
//!                                      └─ TempoExporter
//!
//! propagation::{parse_traceparent, format_traceparent, TraceState}
//! tenant::{tenant_from_headers, inject_tenant}
//! ```
//!
//! See `examples/quickstart.rs` (out-of-tree) for a full wiring example.
//! Tail sampling lives next to head sampling in `sampling::TailSampler`.

pub mod batch;
pub mod exporter;
pub mod id;
pub mod propagation;
pub mod sampling;
pub mod tenant;
pub mod tracer;
pub mod types;

pub use batch::{BatchConfig, BatchSpanProcessor, BatchStats};
pub use exporter::{
    ExportError, ExportResult, InMemoryExporter, NoopExporter, OtlpHttpExporter, SpanExporter,
    TempoExporter,
};
pub use propagation::{
    extract_or_new, format_traceparent, inject, parse_traceparent, parse_tracestate,
    PropagationError, TraceState, TRACEPARENT, TRACESTATE,
};
pub use sampling::{
    AlwaysOff, AlwaysOn, AttrEqualPolicy, ErrorPolicy, LatencyPolicy, ParentBased, Sampler,
    SamplingDecision, SamplingResult, TailPolicy, TailSampler, TraceIdRatioBased,
};
pub use tenant::{filter_by_tenant, inject_tenant, tenant_from_headers, X_SCOPE_ORG_ID};
pub use tracer::{InMemoryProcessor, Span, SpanBuilder, SpanProcessor, Tracer, TracerProvider, TracerProviderBuilder};
pub use types::{
    format_span_id, format_trace_id, parse_span_id, parse_trace_id, AttrValue, Attributes, Event,
    Link, SpanContext, SpanData, SpanKind, SpanId, Status, TraceId, DEFAULT_TENANT, TENANT_LABEL,
};

pub const MODULE_NAME: &str = "tracing";
