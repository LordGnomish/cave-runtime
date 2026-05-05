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
//!                 ├─ Sampler   (AlwaysOn / AlwaysOff / TraceIdRatioBased / ParentBased)
//!                 ├─ SpanProcessor (BatchSpanProcessor / InMemoryProcessor)
//!                  └─ Resource + tenant
//!
//! BatchSpanProcessor ─ async ticker ─ SpanExporter
//!                                       ├─ NoopExporter
//!                                       ├─ InMemoryExporter (tests)
//!                                       ├─ OtlpHttpExporter
//!                                       └─ TempoExporter
//!
//! propagation::{parse_traceparent, format_traceparent, TraceState}
//! tenant::{tenant_from_headers, inject_tenant}
//! ```
//!
//! See `examples/quickstart.rs` (out-of-tree) for a full wiring example.
//! Tail sampling lives next to head sampling in `sampling::TailSampler`.

/// Re-export of the batch processing module.
///
/// Contains `BatchSpanProcessor` and related configuration types like
/// `BatchConfig` and `BatchStats`.
pub mod batch;

/// Re-export of the exporter module.
///
/// Contains trait `SpanExporter` and implementations like `NoopExporter`,
/// `InMemoryExporter`, `OtlpHttpExporter`, and `TempoExporter`.
pub mod exporter;

/// Re-export of the ID generation module.
///
/// Provides utilities for generating and manipulating trace and span IDs.
pub mod id;

/// Re-export of the propagation module.
///
/// Contains functions for parsing and formatting traceparent/tracestate headers,
/// and the `TraceState` type.
pub mod propagation;

/// Re-export of the sampling module.
///
/// Contains sampler implementations (`AlwaysOn`, `AlwaysOff`, etc.) and
/// tail sampling logic (`TailSampler`).
pub mod sampling;

/// Re-export of the tenant module.
///
/// Contains functions for extracting tenant information from headers and
/// injecting it into span attributes.
pub mod tenant;

/// Re-export of the tracer module.
///
/// Contains the core `Tracer`, `TracerProvider`, `SpanBuilder`, and
/// `SpanProcessor` types.
pub mod tracer;

/// Re-export of the types module.
///
/// Contains fundamental types like `Span`, `SpanContext`, `Attributes`,
/// `Event`, `Link`, and ID types.
pub mod types;

/// Re-export of `BatchConfig`, `BatchSpanProcessor`, and `BatchStats`.
///
/// These types are used to configure and run batch span processing.
pub use batch::{BatchConfig, BatchSpanProcessor, BatchStats};

/// Re-export of exporter types and implementations.
///
/// Includes the `SpanExporter` trait and concrete exporters such as
/// `NoopExporter`, `InMemoryExporter`, `OtlpHttpExporter`, and `TempoExporter`.
pub use exporter::{
    ExportError, ExportResult, InMemoryExporter, NoopExporter, OtlpHttpExporter, SpanExporter,
    TempoExporter,
};

/// Re-export of propagation utilities and types.
///
/// Includes functions for extracting, injecting, and formatting trace context
/// headers, as well as the `TraceState` type and related constants.
pub use propagation::{
    extract_or_new, format_traceparent, inject, parse_traceparent, parse_tracestate,
    PropagationError, TraceState, TRACEPARENT, TRACESTATE,
};

/// Re-export of sampling policies and types.
///
/// Includes samplers like `AlwaysOn`, `AlwaysOff`, `TraceIdRatioBased`,
/// `ParentBased`, and tail sampling policies.
pub use sampling::{
    AlwaysOff, AlwaysOn, AttrEqualPolicy, ErrorPolicy, LatencyPolicy, ParentBased, Sampler,
    SamplingDecision, SamplingResult, TailPolicy, TailSampler, TraceIdRatioBased,
};

/// Re-export of tenant-related utilities.
///
/// Includes functions for filtering by tenant, injecting tenant labels,
/// and extracting tenant IDs from headers.
pub use tenant::{filter_by_tenant, inject_tenant, tenant_from_headers, X_SCOPE_ORG_ID};

/// Re-export of tracer and processor types.
///
/// Includes `Tracer`, `TracerProvider`, `SpanBuilder`, `SpanProcessor`,
/// and `InMemoryProcessor`.
pub use tracer::{InMemoryProcessor, Span, SpanBuilder, SpanProcessor, Tracer, TracerProvider, TracerProviderBuilder};

/// Re-export of fundamental types and utility functions.
///
/// Includes ID formatting/parsing functions, attribute types, span data
/// structures, and constants like `DEFAULT_TENANT`.
pub use types::{
    format_span_id, format_trace_id, parse_span_id, parse_trace_id, AttrValue, Attributes, Event,
    Link, SpanContext, SpanData, SpanKind, SpanId, Status, TraceId, DEFAULT_TENANT, TENANT_LABEL,
};

/// The name of the tracing module.
///
/// This constant is used to identify the tracing subsystem in logs and metrics.
pub const MODULE_NAME: &str = "tracing";
