// SPDX-License-Identifier: AGPL-3.0-or-later
//! Zipkin v2 ingestion — JSON and protobuf (proto requires prost build).
//!
//! Zipkin v2 JSON format: https://zipkin.io/zipkin-api/#/default/post_spans

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use crate::ingestion::{normalise_service, us_to_ns};
use crate::types::{Span, SpanEvent, SpanId, SpanKind, SpanStatus, TagValue, TraceId};
use crate::{Result, TraceError};

// ─── Zipkin v2 JSON wire types ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipkinSpan {
    /// 32 or 16 hex char trace ID.
    pub trace_id: String,
    /// 16 hex char span ID.
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    /// SERVER | CLIENT | PRODUCER | CONSUMER (absent = internal)
    pub kind: Option<String>,

    /// Epoch microseconds.
    pub timestamp: Option<i64>,
    /// Duration in microseconds.
    pub duration: Option<i64>,

    pub local_endpoint: Option<ZipkinEndpoint>,
    pub remote_endpoint: Option<ZipkinEndpoint>,

    pub annotations: Option<Vec<ZipkinAnnotation>>,
    /// String → string tags only (Zipkin v2 simplification).
    pub tags: Option<HashMap<String, String>>,

    pub debug: Option<bool>,
    pub shared: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipkinEndpoint {
    pub service_name: Option<String>,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct ZipkinAnnotation {
    /// Epoch microseconds.
    pub timestamp: i64,
    pub value: String,
}

// ─── Parse ─────────────────────────────────────────────────────────────────

/// Parse a Zipkin v2 JSON span array.
pub fn parse_zipkin_json(body: &[u8], tenant_id: &str) -> Result<Vec<Span>> {
    let zspans: Vec<ZipkinSpan> = serde_json::from_slice(body)
        .map_err(|e| TraceError::ParseError(format!("Zipkin JSON: {}", e)))?;

    let spans = zspans
        .into_iter()
        .filter_map(|z| convert_zipkin_span(z, tenant_id))
        .collect();

    Ok(spans)
}

fn convert_zipkin_span(z: ZipkinSpan, tenant_id: &str) -> Option<Span> {
    let trace_id = crate::types::parse_trace_id(&z.trace_id).ok()?;
    let span_id  = crate::types::parse_span_id(&z.id).ok()?;

    let parent_span_id = z
        .parent_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|s| crate::types::parse_span_id(s).ok());

    let start_ns = us_to_ns(z.timestamp.unwrap_or(0));
    let dur_ns   = us_to_ns(z.duration.unwrap_or(0));
    let end_ns   = start_ns + dur_ns;

    let service_name = z
        .local_endpoint
        .as_ref()
        .and_then(|ep| ep.service_name.as_deref())
        .map(normalise_service)
        .unwrap_or_else(|| "unknown".into());

    let kind = match z.kind.as_deref() {
        Some("SERVER")   => SpanKind::Server,
        Some("CLIENT")   => SpanKind::Client,
        Some("PRODUCER") => SpanKind::Producer,
        Some("CONSUMER") => SpanKind::Consumer,
        _                => SpanKind::Internal,
    };

    // Zipkin tags are string→string
    let mut tags: HashMap<String, TagValue> = z
        .tags
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, TagValue::String(v)))
        .collect();

    // Remote endpoint metadata as tags
    if let Some(ep) = &z.remote_endpoint {
        if let Some(sn) = &ep.service_name {
            tags.insert("peer.service".into(), TagValue::String(sn.clone()));
        }
        if let Some(ip) = &ep.ipv4 {
            tags.insert("peer.ipv4".into(), TagValue::String(ip.clone()));
        }
        if let Some(ip) = &ep.ipv6 {
            tags.insert("peer.ipv6".into(), TagValue::String(ip.clone()));
        }
        if let Some(port) = ep.port {
            tags.insert("peer.port".into(), TagValue::Int(port as i64));
        }
    }

    // Status: Zipkin uses "error" tag
    let status = if tags.get("error").is_some() {
        SpanStatus::Error
    } else {
        SpanStatus::Ok
    };

    // Local endpoint attributes become resource attributes
    let mut resource_attrs: HashMap<String, TagValue> = HashMap::new();
    if let Some(ep) = &z.local_endpoint {
        resource_attrs.insert("service.name".into(), TagValue::String(service_name.clone()));
        if let Some(ip) = &ep.ipv4 { resource_attrs.insert("net.host.ip".into(), TagValue::String(ip.clone())); }
        if let Some(port) = ep.port { resource_attrs.insert("net.host.port".into(), TagValue::Int(port as i64)); }
    }

    let log_labels: HashMap<String, String> = resource_attrs.iter()
        .map(|(k, v)| (k.clone(), v.display()))
        .collect();

    // Annotations → span events
    let events: Vec<SpanEvent> = z
        .annotations
        .unwrap_or_default()
        .into_iter()
        .map(|a| SpanEvent {
            time_unix_nano: us_to_ns(a.timestamp),
            name: a.value,
            attributes: HashMap::new(),
        })
        .collect();

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        operation_name: z.name,
        service_name,
        start_time_unix_nano: start_ns,
        end_time_unix_nano: end_ns,
        duration_ns: dur_ns,
        status,
        kind,
        tags,
        events,
        links: vec![],
        resource_attributes: resource_attrs,
        tenant_id: tenant_id.to_owned(),
        baggage: HashMap::new(),
        log_labels,
    })
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const ZIPKIN_V2: &str = r#"[{
  "traceId": "0af7651916cd43dd8448eb211c80319c",
  "id":      "b7ad6b7169203331",
  "parentId": "0000000000000000",
  "name": "get /api/users",
  "kind": "SERVER",
  "timestamp": 1640000000000000,
  "duration": 5000,
  "localEndpoint": {
    "serviceName": "frontend",
    "ipv4": "10.0.0.1",
    "port": 8080
  },
  "remoteEndpoint": {
    "serviceName": "backend",
    "ipv4": "10.0.0.2",
    "port": 9090
  },
  "annotations": [
    {"timestamp": 1640000000001000, "value": "cs"},
    {"timestamp": 1640000000004000, "value": "cr"}
  ],
  "tags": {
    "http.method": "GET",
    "http.status_code": "200"
  }
}]"#;

    #[test]
    fn parse_zipkin_basic() {
        let spans = parse_zipkin_json(ZIPKIN_V2.as_bytes(), "default").unwrap();
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.service_name, "frontend");
        assert_eq!(s.operation_name, "get /api/users");
        assert_eq!(s.kind, SpanKind::Server);
        assert_eq!(s.duration_ns, 5_000_000);
        assert_eq!(s.events.len(), 2);
        assert_eq!(s.events[0].name, "cs");
    }

    #[test]
    fn parse_zipkin_remote_endpoint_tags() {
        let spans = parse_zipkin_json(ZIPKIN_V2.as_bytes(), "default").unwrap();
        let s = &spans[0];
        assert_eq!(s.tags.get("peer.service"), Some(&TagValue::String("backend".into())));
        assert_eq!(s.tags.get("peer.port"), Some(&TagValue::Int(9090)));
    }

    #[test]
    fn parse_zipkin_error_status() {
        let json = r#"[{"traceId":"aabb","id":"1122","name":"op","timestamp":1000,"duration":100,"localEndpoint":{"serviceName":"svc"},"tags":{"error":"timeout"}}]"#;
        let spans = parse_zipkin_json(json.as_bytes(), "default").unwrap();
        assert_eq!(spans[0].status, SpanStatus::Error);
    }

    #[test]
    fn parse_empty_array() {
        let spans = parse_zipkin_json(b"[]", "default").unwrap();
        assert!(spans.is_empty());
    }
}
