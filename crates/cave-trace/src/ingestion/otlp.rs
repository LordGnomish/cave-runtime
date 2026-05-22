// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! OTLP ingestion.
//!
//! Supported encodings
//! ───────────────────
//! • HTTP/JSON  (`Content-Type: application/json`)          — fully implemented
//! • HTTP/proto (`Content-Type: application/x-protobuf`)   — returns 501 (needs prost build)
//! • gRPC       (`Content-Type: application/grpc+proto`)   — returns 501 (needs tonic build)
//!
//! OTLP JSON format mirrors the protobuf JSON encoding defined in
//! opentelemetry-proto/opentelemetry/proto/trace/v1/trace.proto.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ingestion::{normalise_service, us_to_ns};
use crate::types::{Span, SpanEvent, SpanId, SpanKind, SpanLink, SpanStatus, TagValue, TraceId};
use crate::{Result, TraceError};

// ─── OTLP JSON wire types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTraceServiceRequest {
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSpans {
    pub resource: Option<Resource>,
    pub scope_spans: Option<Vec<ScopeSpans>>,
    /// Legacy field name.
    pub instrumentation_library_spans: Option<Vec<ScopeSpans>>,
    pub schema_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Resource {
    pub attributes: Option<Vec<KeyValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeSpans {
    pub scope: Option<Scope>,
    pub spans: Option<Vec<OtlpSpan>>,
}

#[derive(Debug, Deserialize)]
pub struct Scope {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpSpan {
    /// Base64-encoded 16-byte trace ID, or 32-char hex.
    pub trace_id: String,
    /// Base64-encoded 8-byte span ID, or 16-char hex.
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: Option<i32>,
    /// Nanosecond epoch as string (protobuf int64 JSON → string).
    pub start_time_unix_nano: Option<serde_json::Value>,
    pub end_time_unix_nano: Option<serde_json::Value>,
    pub attributes: Option<Vec<KeyValue>>,
    pub events: Option<Vec<OtlpEvent>>,
    pub links: Option<Vec<OtlpLink>>,
    pub status: Option<OtlpStatus>,
    pub trace_state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpEvent {
    pub time_unix_nano: Option<serde_json::Value>,
    pub name: String,
    pub attributes: Option<Vec<KeyValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtlpLink {
    pub trace_id: String,
    pub span_id: String,
    pub trace_state: Option<String>,
    pub attributes: Option<Vec<KeyValue>>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpStatus {
    pub code: Option<i32>,
    pub message: Option<String>,
}

/// OTLP `AnyValue` wrapper.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValue {
    pub key: String,
    pub value: Option<AnyValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnyValue {
    pub string_value: Option<String>,
    pub bool_value: Option<bool>,
    pub int_value: Option<serde_json::Value>,
    pub double_value: Option<f64>,
    pub array_value: Option<ArrayValue>,
    pub kvlist_value: Option<KvListValue>,
    pub bytes_value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ArrayValue {
    pub values: Option<Vec<AnyValue>>,
}

#[derive(Debug, Deserialize)]
pub struct KvListValue {
    pub values: Option<Vec<KeyValue>>,
}

// ─── Conversion ────────────────────────────────────────────────────────────

/// Parse an OTLP HTTP/JSON export request body into Cave spans.
pub fn parse_otlp_json(body: &[u8], tenant_id: &str) -> Result<Vec<Span>> {
    let req: ExportTraceServiceRequest = serde_json::from_slice(body)?;
    let mut out = Vec::new();

    for rs in req.resource_spans {
        let resource_attrs = rs
            .resource
            .as_ref()
            .and_then(|r| r.attributes.as_deref())
            .map(parse_attributes)
            .unwrap_or_default();

        let service_name = resource_attrs
            .get("service.name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .unwrap_or_else(|| "unknown".into());

        // Build log labels from resource attributes for correlation
        let log_labels: HashMap<String, String> = resource_attrs
            .iter()
            .map(|(k, v)| (k.clone(), v.display()))
            .collect();

        let scope_spans_iter = rs
            .scope_spans
            .or(rs.instrumentation_library_spans)
            .unwrap_or_default();

        for ss in scope_spans_iter {
            for span in ss.spans.unwrap_or_default() {
                match convert_span(span, &service_name, &resource_attrs, &log_labels, tenant_id) {
                    Ok(s) => out.push(s),
                    Err(e) => tracing::warn!("OTLP span parse error: {}", e),
                }
            }
        }
    }

    Ok(out)
}

fn convert_span(
    s: OtlpSpan,
    service_name: &str,
    resource_attrs: &HashMap<String, TagValue>,
    log_labels: &HashMap<String, String>,
    tenant_id: &str,
) -> Result<Span> {
    let trace_id = crate::types::parse_trace_id(&s.trace_id)?;
    let span_id = crate::types::parse_span_id(&s.span_id)?;

    let parent_span_id = s
        .parent_span_id
        .as_deref()
        .filter(|p| !p.is_empty() && *p != "0000000000000000")
        .map(crate::types::parse_span_id)
        .transpose()?;

    let start_ns = parse_nano_ts(&s.start_time_unix_nano);
    let end_ns = parse_nano_ts(&s.end_time_unix_nano);

    let tags = s
        .attributes
        .as_deref()
        .map(parse_attributes)
        .unwrap_or_default();
    let events = s
        .events
        .unwrap_or_default()
        .into_iter()
        .map(parse_event)
        .collect();
    let links = s
        .links
        .unwrap_or_default()
        .into_iter()
        .map(parse_link)
        .filter_map(|r| r.ok())
        .collect();

    let status = match s.status.as_ref().and_then(|st| st.code) {
        Some(1) => SpanStatus::Ok,
        Some(2) => SpanStatus::Error,
        _ => SpanStatus::Unset,
    };

    let kind = match s.kind.unwrap_or(0) {
        1 => SpanKind::Internal,
        2 => SpanKind::Server,
        3 => SpanKind::Client,
        4 => SpanKind::Producer,
        5 => SpanKind::Consumer,
        _ => SpanKind::Internal,
    };

    Ok(Span {
        trace_id,
        span_id,
        parent_span_id,
        operation_name: s.name,
        service_name: normalise_service(service_name),
        start_time_unix_nano: start_ns,
        end_time_unix_nano: end_ns,
        duration_ns: end_ns.saturating_sub(start_ns),
        status,
        kind,
        tags,
        events,
        links,
        resource_attributes: resource_attrs.clone(),
        tenant_id: tenant_id.to_owned(),
        baggage: HashMap::new(),
        log_labels: log_labels.clone(),
    })
}

fn parse_nano_ts(v: &Option<serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn parse_attributes(kvs: &[KeyValue]) -> HashMap<String, TagValue> {
    kvs.iter()
        .filter_map(|kv| {
            kv.value
                .as_ref()
                .map(|v| (kv.key.clone(), any_value_to_tag(v)))
        })
        .collect()
}

fn any_value_to_tag(v: &AnyValue) -> TagValue {
    if let Some(s) = &v.string_value {
        return TagValue::String(s.clone());
    }
    if let Some(b) = v.bool_value {
        return TagValue::Bool(b);
    }
    if let Some(d) = v.double_value {
        return TagValue::Float(d);
    }
    if let Some(n) = &v.int_value {
        if let Some(i) = n.as_i64() {
            return TagValue::Int(i);
        }
        if let Some(s) = n.as_str() {
            if let Ok(i) = s.parse::<i64>() {
                return TagValue::Int(i);
            }
        }
    }
    if let Some(bs) = &v.bytes_value {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        if let Ok(bytes) = STANDARD.decode(bs) {
            return TagValue::Binary(bytes);
        }
    }
    if let Some(arr) = &v.array_value {
        let elems = arr.values.as_deref().unwrap_or(&[]);
        let s: Vec<String> = elems
            .iter()
            .map(|av| any_value_to_tag(av).display())
            .collect();
        return TagValue::String(format!("[{}]", s.join(",")));
    }
    TagValue::String(String::new())
}

fn parse_event(e: OtlpEvent) -> SpanEvent {
    SpanEvent {
        time_unix_nano: parse_nano_ts(&e.time_unix_nano),
        name: e.name,
        attributes: e
            .attributes
            .as_deref()
            .map(parse_attributes)
            .unwrap_or_default(),
    }
}

fn parse_link(l: OtlpLink) -> Result<SpanLink> {
    Ok(SpanLink {
        trace_id: crate::types::parse_trace_id(&l.trace_id)?,
        span_id: crate::types::parse_span_id(&l.span_id)?,
        trace_state: l.trace_state.unwrap_or_default(),
        attributes: l
            .attributes
            .as_deref()
            .map(parse_attributes)
            .unwrap_or_default(),
    })
}

// ─── OTLP HTTP/JSON export response ───────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTraceServiceResponse {
    pub partial_success: PartialSuccess,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialSuccess {
    pub rejected_spans: i64,
    pub error_message: String,
}

impl ExportTraceServiceResponse {
    pub fn ok() -> Self {
        ExportTraceServiceResponse {
            partial_success: PartialSuccess {
                rejected_spans: 0,
                error_message: String::new(),
            },
        }
    }

    pub fn partial(rejected: i64, msg: &str) -> Self {
        ExportTraceServiceResponse {
            partial_success: PartialSuccess {
                rejected_spans: rejected,
                error_message: msg.to_owned(),
            },
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const OTLP_JSON: &str = r#"{
  "resourceSpans": [{
    "resource": {
      "attributes": [
        {"key": "service.name", "value": {"stringValue": "my-service"}},
        {"key": "host.name",    "value": {"stringValue": "web-01"}}
      ]
    },
    "scopeSpans": [{
      "scope": {"name": "my-scope", "version": "1.0"},
      "spans": [{
        "traceId": "0af7651916cd43dd8448eb211c80319c",
        "spanId":  "b7ad6b7169203331",
        "parentSpanId": "",
        "name": "GET /api/users",
        "kind": 2,
        "startTimeUnixNano": "1640000000000000000",
        "endTimeUnixNano":   "1640000000005000000",
        "attributes": [
          {"key": "http.method", "value": {"stringValue": "GET"}},
          {"key": "http.status_code", "value": {"intValue": "200"}}
        ],
        "status": {"code": 1, "message": "OK"},
        "events": [{
          "timeUnixNano": "1640000000001000000",
          "name": "cache_miss",
          "attributes": [{"key": "cache.key", "value": {"stringValue": "users:42"}}]
        }]
      }]
    }]
  }]
}"#;

    #[test]
    fn parse_otlp_json_basic() {
        let spans = parse_otlp_json(OTLP_JSON.as_bytes(), "default").unwrap();
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.service_name, "my-service");
        assert_eq!(span.operation_name, "GET /api/users");
        assert_eq!(span.status, SpanStatus::Ok);
        assert_eq!(span.kind, SpanKind::Server);
        assert!(span.parent_span_id.is_none());
        assert_eq!(span.duration_ns, 5_000_000);
        assert_eq!(span.events.len(), 1);
        assert_eq!(span.events[0].name, "cache_miss");
    }

    #[test]
    fn parse_otlp_json_tag_types() {
        let spans = parse_otlp_json(OTLP_JSON.as_bytes(), "default").unwrap();
        let span = &spans[0];
        assert_eq!(
            span.tags.get("http.method"),
            Some(&TagValue::String("GET".into()))
        );
        assert_eq!(span.tags.get("http.status_code"), Some(&TagValue::Int(200)));
    }

    #[test]
    fn parse_otlp_json_resource_attrs() {
        let spans = parse_otlp_json(OTLP_JSON.as_bytes(), "default").unwrap();
        let span = &spans[0];
        assert_eq!(
            span.resource_attributes.get("host.name"),
            Some(&TagValue::String("web-01".into()))
        );
    }

    #[test]
    fn parse_invalid_json_errors() {
        let result = parse_otlp_json(b"not json", "default");
        assert!(result.is_err());
    }
}
