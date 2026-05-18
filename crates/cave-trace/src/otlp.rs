// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::error::*;
use crate::types::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// OTLP JSON structures (simplified subset of the full proto-JSON format)
#[derive(Debug, Deserialize)]
pub struct OtlpExportRequest {
    pub resource_spans: Vec<ResourceSpans>,
}

#[derive(Debug, Deserialize)]
pub struct ResourceSpans {
    pub resource: Option<OtlpResource>,
    pub scope_spans: Vec<ScopeSpans>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpResource {
    pub attributes: Option<Vec<OtlpKeyValue>>,
}

#[derive(Debug, Deserialize)]
pub struct ScopeSpans {
    pub scope: Option<OtlpScope>,
    pub spans: Vec<OtlpSpan>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpScope {
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: Option<i32>,
    pub start_time_unix_nano: String,
    pub end_time_unix_nano: String,
    pub attributes: Option<Vec<OtlpKeyValue>>,
    pub events: Option<Vec<OtlpEvent>>,
    pub links: Option<Vec<OtlpLink>>,
    pub status: Option<OtlpStatus>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OtlpKeyValue {
    pub key: String,
    pub value: OtlpAnyValue,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OtlpAnyValue {
    pub string_value: Option<String>,
    pub bool_value: Option<bool>,
    pub int_value: Option<i64>,
    pub double_value: Option<f64>,
    pub array_value: Option<OtlpArrayValue>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct OtlpArrayValue {
    pub values: Vec<OtlpAnyValue>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpEvent {
    pub name: String,
    pub time_unix_nano: Option<String>,
    pub attributes: Option<Vec<OtlpKeyValue>>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpLink {
    pub trace_id: String,
    pub span_id: String,
    pub attributes: Option<Vec<OtlpKeyValue>>,
}

#[derive(Debug, Deserialize)]
pub struct OtlpStatus {
    pub code: Option<i32>,
    pub message: Option<String>,
}

pub struct OtlpReceiver;

impl OtlpReceiver {
    /// Parse OTLP JSON export request and convert to our Span type
    pub fn parse_export(json: &str) -> TraceResult<Vec<Span>> {
        let req: OtlpExportRequest =
            serde_json::from_str(json).map_err(|e| TraceError::OtlpError(e.to_string()))?;

        let mut spans = vec![];
        for rs in req.resource_spans {
            let service_name = rs
                .resource
                .as_ref()
                .and_then(|r| r.attributes.as_ref())
                .and_then(|attrs| attrs.iter().find(|kv| kv.key == "service.name"))
                .and_then(|kv| kv.value.string_value.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let resource_attrs = Self::convert_attrs(
                rs.resource
                    .as_ref()
                    .and_then(|r| r.attributes.as_ref())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]),
            );

            for ss in rs.scope_spans {
                for otlp_span in ss.spans {
                    let span =
                        Self::convert_span(otlp_span, &service_name, resource_attrs.clone())?;
                    spans.push(span);
                }
            }
        }
        Ok(spans)
    }

    fn convert_span(
        s: OtlpSpan,
        service_name: &str,
        resource_attributes: HashMap<String, AttributeValue>,
    ) -> TraceResult<Span> {
        let start_ns: i64 = s.start_time_unix_nano.parse().unwrap_or(0);
        let end_ns: i64 = s.end_time_unix_nano.parse().unwrap_or(0);
        let duration_us = (end_ns - start_ns) / 1000;

        let start_time =
            chrono::DateTime::from_timestamp(start_ns / 1_000_000_000, (start_ns % 1_000_000_000) as u32)
                .unwrap_or_else(Utc::now);
        let end_time =
            chrono::DateTime::from_timestamp(end_ns / 1_000_000_000, (end_ns % 1_000_000_000) as u32)
                .unwrap_or_else(Utc::now);

        let kind = match s.kind.unwrap_or(0) {
            1 => SpanKind::Internal,
            2 => SpanKind::Server,
            3 => SpanKind::Client,
            4 => SpanKind::Producer,
            5 => SpanKind::Consumer,
            _ => SpanKind::Unspecified,
        };
        let status = match s.status.as_ref().and_then(|st| st.code) {
            Some(1) => SpanStatus::Ok,
            Some(2) => SpanStatus::Error,
            _ => SpanStatus::Unset,
        };

        Ok(Span {
            trace_id: s.trace_id,
            span_id: s.span_id.clone(),
            parent_span_id: s.parent_span_id.filter(|p| !p.is_empty()),
            operation_name: s.name,
            service_name: service_name.to_string(),
            start_time,
            end_time,
            duration_us,
            status,
            kind,
            tags: Self::convert_attrs(s.attributes.as_deref().unwrap_or(&[])),
            events: s
                .events
                .unwrap_or_default()
                .into_iter()
                .map(|e| {
                    let ts_ns: i64 = e
                        .time_unix_nano
                        .as_deref()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0);
                    SpanEvent {
                        name: e.name,
                        timestamp: chrono::DateTime::from_timestamp(
                            ts_ns / 1_000_000_000,
                            (ts_ns % 1_000_000_000) as u32,
                        )
                        .unwrap_or_else(Utc::now),
                        attributes: Self::convert_attrs(e.attributes.as_deref().unwrap_or(&[])),
                    }
                })
                .collect(),
            links: s
                .links
                .unwrap_or_default()
                .into_iter()
                .map(|l| SpanLink {
                    trace_id: l.trace_id,
                    span_id: l.span_id,
                    attributes: Self::convert_attrs(l.attributes.as_deref().unwrap_or(&[])),
                })
                .collect(),
            resource_attributes,
        })
    }

    fn convert_attrs(kvs: &[OtlpKeyValue]) -> HashMap<String, AttributeValue> {
        kvs.iter()
            .map(|kv| {
                let val = if let Some(s) = &kv.value.string_value {
                    AttributeValue::String(s.clone())
                } else if let Some(b) = kv.value.bool_value {
                    AttributeValue::Bool(b)
                } else if let Some(i) = kv.value.int_value {
                    AttributeValue::Int(i)
                } else if let Some(d) = kv.value.double_value {
                    AttributeValue::Double(d)
                } else {
                    AttributeValue::String(String::new())
                };
                (kv.key.clone(), val)
            })
            .collect()
    }

    /// Build a simple OTLP export JSON for testing
    pub fn build_test_export(
        service: &str,
        op: &str,
        trace_id: &str,
        span_id: &str,
        duration_us: i64,
    ) -> String {
        let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        serde_json::json!({
            "resource_spans": [{
                "resource": {
                    "attributes": [{"key": "service.name", "value": {"string_value": service}}]
                },
                "scope_spans": [{
                    "spans": [{
                        "trace_id": trace_id,
                        "span_id": span_id,
                        "name": op,
                        "kind": 2,
                        "start_time_unix_nano": (now_ns - duration_us * 1000).to_string(),
                        "end_time_unix_nano": now_ns.to_string(),
                        "status": {"code": 1}
                    }]
                }]
            }]
        })
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otlp_parse_export() {
        let now_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let json = serde_json::json!({
            "resource_spans": [{
                "resource": {
                    "attributes": [
                        {"key": "service.name", "value": {"string_value": "my-service"}},
                        {"key": "host.name", "value": {"string_value": "server-01"}}
                    ]
                },
                "scope_spans": [{
                    "spans": [{
                        "trace_id": "abc123",
                        "span_id": "span001",
                        "name": "http.request",
                        "kind": 2,
                        "start_time_unix_nano": (now_ns - 5_000_000i64).to_string(),
                        "end_time_unix_nano": now_ns.to_string(),
                        "attributes": [
                            {"key": "http.method", "value": {"string_value": "GET"}},
                            {"key": "http.status_code", "value": {"int_value": 200}}
                        ],
                        "status": {"code": 1, "message": "OK"}
                    }]
                }]
            }]
        }).to_string();

        let spans = OtlpReceiver::parse_export(&json).unwrap();
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.service_name, "my-service");
        assert_eq!(span.trace_id, "abc123");
        assert_eq!(span.span_id, "span001");
        assert_eq!(span.operation_name, "http.request");
        assert_eq!(span.kind, SpanKind::Server);
        assert_eq!(span.status, SpanStatus::Ok);
        assert_eq!(span.duration_us, 5000);
        assert!(span.tags.contains_key("http.method"));
        // resource attributes should carry over
        assert!(span.resource_attributes.contains_key("service.name"));
    }

    #[test]
    fn otlp_build_and_parse() {
        let json = OtlpReceiver::build_test_export("test-svc", "GET /ping", "trace1", "span1", 1000);
        let spans = OtlpReceiver::parse_export(&json).unwrap();
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.service_name, "test-svc");
        assert_eq!(span.operation_name, "GET /ping");
        assert_eq!(span.trace_id, "trace1");
        assert_eq!(span.span_id, "span1");
        assert_eq!(span.status, SpanStatus::Ok);
    }
}
