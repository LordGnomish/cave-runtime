// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Request span → OTLP trace export — userspace model of grafana/beyla
//! `pkg/internal/request` (Span construction) and `pkg/internal/export/otel`
//! (OTLP rendering).
//!
//! Beyla turns each detected HTTP / gRPC / SQL request into a `request.Span`
//! decorated with OpenTelemetry semantic-convention attributes, then ships
//! batches as OTLP `ResourceSpans`. This module ports that span shape and
//! the OTLP/JSON rendering — the same payload Beyla's OTLP exporter sends,
//! built from the [`crate::discover`] detection output.

use serde_json::{json, Value};

/// OTLP span kind (numeric codes per the OTLP proto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Internal = 1,
    Server = 2,
    Client = 3,
    Producer = 4,
    Consumer = 5,
}

/// OTLP status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Unset = 0,
    Ok = 1,
    Error = 2,
}

/// Span attribute value (the subset Beyla emits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrValue {
    Str(String),
    Int(i64),
}

impl AttrValue {
    fn to_otlp(&self) -> Value {
        match self {
            // OTLP/JSON encodes int64 as a string.
            AttrValue::Str(s) => json!({ "stringValue": s }),
            AttrValue::Int(i) => json!({ "intValue": i.to_string() }),
        }
    }
}

/// A trace span built from a detected request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent_span_id: Option<[u8; 8]>,
    pub name: String,
    pub kind: SpanKind,
    pub start_ns: u64,
    pub end_ns: u64,
    pub status: StatusCode,
    pub attributes: Vec<(String, AttrValue)>,
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

impl Span {
    fn base(name: String, kind: SpanKind, start_ns: u64, end_ns: u64) -> Self {
        Span {
            trace_id: [0; 16],
            span_id: [0; 8],
            parent_span_id: None,
            name,
            kind,
            start_ns,
            end_ns,
            status: StatusCode::Unset,
            attributes: Vec::new(),
        }
    }

    /// Build an HTTP server span. Per OTel HTTP semconv, only 5xx marks the
    /// *server* span as an error (4xx is a client fault).
    pub fn http_server(method: &str, route: &str, status_code: u16, start_ns: u64, end_ns: u64) -> Span {
        let name = format!("{method} {route}");
        let mut s = Span::base(name, SpanKind::Server, start_ns, end_ns);
        s.attributes.push((
            "http.request.method".into(),
            AttrValue::Str(method.to_string()),
        ));
        s.attributes
            .push(("url.path".into(), AttrValue::Str(route.to_string())));
        s.attributes.push((
            "http.response.status_code".into(),
            AttrValue::Int(status_code as i64),
        ));
        if status_code >= 500 {
            s.status = StatusCode::Error;
        }
        s
    }

    /// Build a gRPC server span. A non-zero gRPC status is an error.
    pub fn grpc(service: &str, method: &str, grpc_status: i64, start_ns: u64, end_ns: u64) -> Span {
        let name = format!("{service}/{method}");
        let mut s = Span::base(name, SpanKind::Server, start_ns, end_ns);
        s.attributes
            .push(("rpc.system".into(), AttrValue::Str("grpc".into())));
        s.attributes
            .push(("rpc.service".into(), AttrValue::Str(service.to_string())));
        s.attributes
            .push(("rpc.method".into(), AttrValue::Str(method.to_string())));
        s.attributes
            .push(("rpc.grpc.status_code".into(), AttrValue::Int(grpc_status)));
        if grpc_status != 0 {
            s.status = StatusCode::Error;
        }
        s
    }

    /// Build a database client span.
    pub fn db(system: &str, command: &str, statement: &str, start_ns: u64, end_ns: u64) -> Span {
        let mut s = Span::base(command.to_string(), SpanKind::Client, start_ns, end_ns);
        s.attributes
            .push(("db.system".into(), AttrValue::Str(system.to_string())));
        s.attributes
            .push(("db.operation".into(), AttrValue::Str(command.to_string())));
        s.attributes
            .push(("db.statement".into(), AttrValue::Str(statement.to_string())));
        s
    }

    /// Span duration in nanoseconds (saturating).
    pub fn duration_ns(&self) -> u64 {
        self.end_ns.saturating_sub(self.start_ns)
    }

    /// Look up an attribute by key.
    pub fn attr(&self, key: &str) -> Option<&AttrValue> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    pub fn trace_id_hex(&self) -> String {
        hex(&self.trace_id)
    }

    pub fn span_id_hex(&self) -> String {
        hex(&self.span_id)
    }

    fn to_otlp_span(&self) -> Value {
        let mut span = json!({
            "traceId": self.trace_id_hex(),
            "spanId": self.span_id_hex(),
            "name": self.name,
            "kind": self.kind as i32,
            "startTimeUnixNano": self.start_ns.to_string(),
            "endTimeUnixNano": self.end_ns.to_string(),
            "attributes": self.attributes.iter().map(|(k, v)| {
                json!({ "key": k, "value": v.to_otlp() })
            }).collect::<Vec<_>>(),
            "status": { "code": self.status as i32 },
        });
        if let Some(parent) = &self.parent_span_id {
            span["parentSpanId"] = json!(hex(parent));
        }
        span
    }
}

/// Render a batch of spans for one service as an OTLP/JSON `ResourceSpans`
/// payload — the shape Beyla's OTLP HTTP exporter posts.
pub fn to_otlp_resource_spans(service_name: &str, spans: &[Span]) -> Value {
    json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [{
                    "key": "service.name",
                    "value": { "stringValue": service_name }
                }]
            },
            "scopeSpans": [{
                "scope": { "name": "cave-ebpf-common/beyla" },
                "spans": spans.iter().map(Span::to_otlp_span).collect::<Vec<_>>(),
            }]
        }]
    })
}
