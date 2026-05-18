// SPDX-License-Identifier: AGPL-3.0-or-later
//! OpenCensus ingestion — grpc-json format.
//!
//! OpenCensus is mostly superseded by OpenTelemetry but some legacy clients
//! still emit it. We accept the JSON encoding of the proto3 wire format.
//!
//! Reference: google.golang.org/genproto/googleapis/devtools/cloudtrace/v2

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ingestion::{ms_to_ns, normalise_service, us_to_ns};
use crate::types::{Span, SpanEvent, SpanId, SpanKind, SpanStatus, TagValue, TraceId};
use crate::{Result, TraceError};

// ─── Wire types ────────────────────────────────────────────────────────────

/// OpenCensus ExportTraceServiceRequest (simplified JSON mapping).
#[derive(Debug, Deserialize)]
pub struct OcExportRequest {
    pub node: Option<OcNode>,
    pub resource: Option<OcResource>,
    pub spans: Option<Vec<OcSpan>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcNode {
    pub identifier: Option<OcProcessIdentifier>,
    pub library_info: Option<OcLibraryInfo>,
    pub service_info: Option<OcServiceInfo>,
    pub attributes: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcProcessIdentifier {
    pub host_name: Option<String>,
    pub pid: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct OcLibraryInfo {
    pub language: Option<i32>,
    pub version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OcServiceInfo {
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcResource {
    pub r#type: Option<String>,
    pub labels: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcSpan {
    /// Base64-encoded 16 bytes.
    pub trace_id: Option<String>,
    /// Base64-encoded 8 bytes.
    pub span_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub name: Option<OcTruncatableString>,
    pub kind: Option<i32>,

    /// RFC 3339 timestamp with nanoseconds.
    pub start_time: Option<String>,
    pub end_time: Option<String>,

    pub attributes: Option<OcAttributes>,
    pub time_events: Option<OcTimeEvents>,
    pub links: Option<OcLinks>,
    pub status: Option<OcStatus>,
}

#[derive(Debug, Deserialize)]
pub struct OcTruncatableString {
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcAttributes {
    pub attribute_map: Option<HashMap<String, OcAttributeValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcAttributeValue {
    pub string_value: Option<OcTruncatableString>,
    pub int_value: Option<serde_json::Value>,
    pub bool_value: Option<bool>,
    pub double_value: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcTimeEvents {
    pub time_event: Option<Vec<OcTimeEvent>>,
}

#[derive(Debug, Deserialize)]
pub struct OcTimeEvent {
    pub time: Option<String>,
    pub annotation: Option<OcAnnotation>,
    pub message_event: Option<OcMessageEvent>,
}

#[derive(Debug, Deserialize)]
pub struct OcAnnotation {
    pub description: Option<OcTruncatableString>,
    pub attributes: Option<OcAttributes>,
}

#[derive(Debug, Deserialize)]
pub struct OcMessageEvent {
    pub r#type: Option<i32>,
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OcLinks {
    pub link: Option<Vec<OcLink>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcLink {
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
    pub r#type: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct OcStatus {
    pub code: Option<i32>,
    pub message: Option<String>,
}

// ─── Parse ─────────────────────────────────────────────────────────────────

/// Parse OpenCensus JSON export into Cave spans.
pub fn parse_opencensus_json(body: &[u8], tenant_id: &str) -> Result<Vec<Span>> {
    let req: OcExportRequest = serde_json::from_slice(body)
        .map_err(|e| TraceError::ParseError(format!("OpenCensus JSON: {}", e)))?;

    let service_name = req
        .node
        .as_ref()
        .and_then(|n| n.service_info.as_ref())
        .and_then(|s| s.name.as_deref())
        .map(normalise_service)
        .unwrap_or_else(|| "unknown".into());

    let mut resource_attrs: HashMap<String, TagValue> = HashMap::new();
    resource_attrs.insert("service.name".into(), TagValue::String(service_name.clone()));

    if let Some(node) = &req.node {
        if let Some(id) = &node.identifier {
            if let Some(h) = &id.host_name {
                resource_attrs.insert("host.name".into(), TagValue::String(h.clone()));
            }
        }
        if let Some(attrs) = &node.attributes {
            for (k, v) in attrs {
                resource_attrs.insert(k.clone(), TagValue::String(v.clone()));
            }
        }
    }
    if let Some(res) = &req.resource {
        if let Some(t) = &res.r#type {
            resource_attrs.insert("resource.type".into(), TagValue::String(t.clone()));
        }
        if let Some(labels) = &res.labels {
            for (k, v) in labels {
                resource_attrs.insert(k.clone(), TagValue::String(v.clone()));
            }
        }
    }

    let log_labels: HashMap<String, String> = resource_attrs
        .iter()
        .map(|(k, v)| (k.clone(), v.display()))
        .collect();

    let spans = req
        .spans
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| convert_oc_span(s, &service_name, &resource_attrs, &log_labels, tenant_id))
        .collect();

    Ok(spans)
}

fn convert_oc_span(
    s: OcSpan,
    service_name: &str,
    resource_attrs: &HashMap<String, TagValue>,
    log_labels: &HashMap<String, String>,
    tenant_id: &str,
) -> Option<Span> {
    let trace_id = crate::types::parse_trace_id(s.trace_id.as_deref()?).ok()?;
    let span_id  = crate::types::parse_span_id(s.span_id.as_deref()?).ok()?;

    let parent_span_id = s
        .parent_span_id
        .as_deref()
        .filter(|p| !p.is_empty())
        .and_then(|p| crate::types::parse_span_id(p).ok());

    let start_ns = s.start_time.as_deref().and_then(parse_rfc3339_ns).unwrap_or(0);
    let end_ns   = s.end_time.as_deref().and_then(parse_rfc3339_ns).unwrap_or(start_ns);
    let dur_ns   = end_ns.saturating_sub(start_ns);

    let operation_name = s.name.map(|n| n.value).unwrap_or_else(|| "unknown".into());

    let kind = match s.kind.unwrap_or(0) {
        1 => SpanKind::Server,
        2 => SpanKind::Client,
        _ => SpanKind::Internal,
    };

    let tags: HashMap<String, TagValue> = s
        .attributes
        .as_ref()
        .and_then(|a| a.attribute_map.as_ref())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), oc_attr_to_tag(v))).collect())
        .unwrap_or_default();

    let status = match s.status.as_ref().map(|st| st.code.unwrap_or(0)) {
        Some(0) => SpanStatus::Ok,
        Some(c) if c != 0 => SpanStatus::Error,
        _ => SpanStatus::Unset,
    };

    let events: Vec<SpanEvent> = s
        .time_events
        .as_ref()
        .and_then(|te| te.time_event.as_deref())
        .unwrap_or(&[])
        .iter()
        .map(|te| SpanEvent {
            time_unix_nano: te.time.as_deref().and_then(parse_rfc3339_ns).unwrap_or(start_ns),
            name: te
                .annotation
                .as_ref()
                .and_then(|a| a.description.as_ref())
                .map(|d| d.value.clone())
                .unwrap_or_else(|| "event".into()),
            attributes: te
                .annotation
                .as_ref()
                .and_then(|a| a.attributes.as_ref())
                .and_then(|attrs| attrs.attribute_map.as_ref())
                .map(|m| m.iter().map(|(k, v)| (k.clone(), oc_attr_to_tag(v))).collect())
                .unwrap_or_default(),
        })
        .collect();

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        operation_name,
        service_name: service_name.to_owned(),
        start_time_unix_nano: start_ns,
        end_time_unix_nano: end_ns,
        duration_ns: dur_ns,
        status,
        kind,
        tags,
        events,
        links: vec![],
        resource_attributes: resource_attrs.clone(),
        tenant_id: tenant_id.to_owned(),
        baggage: HashMap::new(),
        log_labels: log_labels.clone(),
    })
}

fn oc_attr_to_tag(v: &OcAttributeValue) -> TagValue {
    if let Some(s) = &v.string_value { return TagValue::String(s.value.clone()); }
    if let Some(b) = v.bool_value    { return TagValue::Bool(b); }
    if let Some(d) = v.double_value  { return TagValue::Float(d); }
    if let Some(n) = &v.int_value {
        if let Some(i) = n.as_i64() { return TagValue::Int(i); }
        if let Some(s) = n.as_str() {
            if let Ok(i) = s.parse::<i64>() { return TagValue::Int(i); }
        }
    }
    TagValue::String(String::new())
}

/// Parse RFC3339 timestamp with optional nanoseconds suffix into epoch ns.
/// e.g. "2022-01-01T00:00:00.000000001Z"
fn parse_rfc3339_ns(s: &str) -> Option<u64> {
    // Use chrono for robust parsing
    let dt = chrono::DateTime::parse_from_rfc3339(s).ok()?;
    let ns = dt.timestamp_nanos_opt()?;
    Some(ns.max(0) as u64)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const OC_JSON: &str = r#"{
  "node": {
    "identifier": {"hostName": "my-host"},
    "serviceInfo": {"name": "my-service"}
  },
  "spans": [{
    "traceId": "AAAAAAAAAAAAAAAAAAAAAA==",
    "spanId":  "AAAAAAAAAAA=",
    "name": {"value": "ListUsers"},
    "kind": 1,
    "startTime": "2022-01-01T00:00:00.000000000Z",
    "endTime":   "2022-01-01T00:00:00.005000000Z",
    "attributes": {
      "attributeMap": {
        "http.method": {"stringValue": {"value": "GET"}},
        "http.status_code": {"intValue": 200}
      }
    },
    "status": {"code": 0, "message": "OK"}
  }]
}"#;

    #[test]
    fn parse_oc_basic() {
        let spans = parse_opencensus_json(OC_JSON.as_bytes(), "default").unwrap();
        assert_eq!(spans.len(), 1);
        let s = &spans[0];
        assert_eq!(s.service_name, "my-service");
        assert_eq!(s.operation_name, "ListUsers");
        assert_eq!(s.kind, SpanKind::Server);
        assert_eq!(s.status, SpanStatus::Ok);
        assert_eq!(s.duration_ns, 5_000_000);
    }

    #[test]
    fn parse_oc_resource_in_resource_attrs() {
        let spans = parse_opencensus_json(OC_JSON.as_bytes(), "default").unwrap();
        assert_eq!(
            spans[0].resource_attributes.get("host.name"),
            Some(&TagValue::String("my-host".into()))
        );
    }

    #[test]
    fn parse_rfc3339_ns_works() {
        let ns = parse_rfc3339_ns("2022-01-01T00:00:01.000000001Z").unwrap();
        assert_eq!(ns % 1_000_000_000, 1); // last ns digit
    }
}
