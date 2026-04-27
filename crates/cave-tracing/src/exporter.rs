//! Span exporters.
//!
//! - `InMemoryExporter` — collects spans in a vector for tests.
//! - `OtlpHttpExporter` — POSTs spans as OTLP/HTTP-JSON to an OTel collector.
//! - `TempoExporter` — variant that hits Tempo's `/api/push` endpoint.
//! - `NoopExporter` — drops everything (used when tracing is disabled).

use crate::types::{SpanData, AttrValue, Status};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("transport: {0}")]
    Transport(String),
    #[error("serialization: {0}")]
    Serialization(String),
    #[error("upstream rejected: status {0}")]
    Rejected(u16),
}

pub type ExportResult = Result<(), ExportError>;

#[async_trait]
pub trait SpanExporter: Send + Sync {
    /// Export a batch of spans. Implementations should be idempotent and
    /// must NOT panic — return ExportError on failure.
    async fn export(&self, batch: Vec<SpanData>) -> ExportResult;

    /// Flush any buffered spans. Default no-op.
    async fn shutdown(&self) -> ExportResult { Ok(()) }
}

// ─── NoopExporter ─────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopExporter;

#[async_trait]
impl SpanExporter for NoopExporter {
    async fn export(&self, _batch: Vec<SpanData>) -> ExportResult { Ok(()) }
}

// ─── InMemoryExporter ─────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct InMemoryExporter {
    inner: Arc<Mutex<Vec<SpanData>>>,
    fail_next: Arc<Mutex<usize>>,
}

impl InMemoryExporter {
    pub fn new() -> Self { Default::default() }

    pub fn collected(&self) -> Vec<SpanData> {
        self.inner.lock().clone()
    }

    pub fn count(&self) -> usize { self.inner.lock().len() }

    /// Force the next `n` exports to fail with `Transport(...)`.
    pub fn fail_next(&self, n: usize) {
        *self.fail_next.lock() = n;
    }
}

#[async_trait]
impl SpanExporter for InMemoryExporter {
    async fn export(&self, batch: Vec<SpanData>) -> ExportResult {
        let mut fail = self.fail_next.lock();
        if *fail > 0 {
            *fail -= 1;
            drop(fail);
            return Err(ExportError::Transport("induced".into()));
        }
        drop(fail);
        self.inner.lock().extend(batch);
        Ok(())
    }
}

// ─── OtlpHttpExporter ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct OtlpHttpExporter {
    pub endpoint: String,
    pub headers: Vec<(String, String)>,
    pub timeout: std::time::Duration,
}

impl OtlpHttpExporter {
    pub fn new(endpoint: impl Into<String>) -> Self {
        OtlpHttpExporter {
            endpoint: endpoint.into(),
            headers: vec![],
            timeout: std::time::Duration::from_secs(10),
        }
    }

    pub fn with_header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }

    /// Render a batch into OTLP/HTTP-JSON shape (resource_spans → scope_spans → spans).
    pub fn render_payload(&self, batch: &[SpanData]) -> Value {
        // Group spans by (tenant_id, resource, scope)
        use std::collections::BTreeMap;
        type Key = (String, String, String); // (tenant, scope, resource_hash)
        let mut groups: BTreeMap<Key, Vec<&SpanData>> = BTreeMap::new();
        for span in batch {
            let mut resource_kvs: Vec<(String, String)> = span.resource.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            resource_kvs.sort();
            let resource_hash = resource_kvs.iter()
                .map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(",");
            groups.entry((span.tenant_id.clone(), span.instrumentation_scope.clone(), resource_hash))
                .or_default()
                .push(span);
        }

        let mut resource_spans: Vec<Value> = Vec::new();
        for ((tenant, scope, _hash), spans) in groups {
            let resource_attrs: Vec<Value> = spans[0].resource.iter()
                .map(|(k, v)| json!({"key": k, "value": {"stringValue": v}}))
                .chain(std::iter::once(json!({
                    "key": crate::types::TENANT_LABEL,
                    "value": {"stringValue": tenant}
                })))
                .collect();

            let scope_spans = json!({
                "scope": { "name": scope },
                "spans": spans.iter().map(|s| span_to_otlp(s)).collect::<Vec<_>>(),
            });

            resource_spans.push(json!({
                "resource": { "attributes": resource_attrs },
                "scopeSpans": [scope_spans],
            }));
        }
        json!({ "resourceSpans": resource_spans })
    }
}

fn attr_to_otlp(value: &AttrValue) -> Value {
    match value {
        AttrValue::String(s) => json!({"stringValue": s}),
        AttrValue::Int(i) => json!({"intValue": i.to_string()}),
        AttrValue::Float(f) => json!({"doubleValue": f}),
        AttrValue::Bool(b) => json!({"boolValue": b}),
        AttrValue::StringArray(a) => json!({"arrayValue": {"values": a.iter().map(|v| json!({"stringValue": v})).collect::<Vec<_>>()}}),
        AttrValue::IntArray(a) => json!({"arrayValue": {"values": a.iter().map(|v| json!({"intValue": v.to_string()})).collect::<Vec<_>>()}}),
        AttrValue::FloatArray(a) => json!({"arrayValue": {"values": a.iter().map(|v| json!({"doubleValue": v})).collect::<Vec<_>>()}}),
        AttrValue::BoolArray(a) => json!({"arrayValue": {"values": a.iter().map(|v| json!({"boolValue": v})).collect::<Vec<_>>()}}),
    }
}

fn span_to_otlp(s: &SpanData) -> Value {
    let attributes: Vec<Value> = s.attributes.iter()
        .map(|(k, v)| json!({"key": k, "value": attr_to_otlp(v)}))
        .collect();
    let events: Vec<Value> = s.events.iter().map(|e| {
        json!({
            "name": e.name,
            "timeUnixNano": e.time.timestamp_nanos_opt().unwrap_or(0).to_string(),
            "attributes": e.attributes.iter()
                .map(|(k, v)| json!({"key": k, "value": attr_to_otlp(v)}))
                .collect::<Vec<_>>(),
        })
    }).collect();
    let links: Vec<Value> = s.links.iter().map(|l| {
        json!({
            "traceId": crate::types::format_trace_id(l.context.trace_id),
            "spanId": crate::types::format_span_id(l.context.span_id),
            "attributes": l.attributes.iter()
                .map(|(k, v)| json!({"key": k, "value": attr_to_otlp(v)}))
                .collect::<Vec<_>>(),
        })
    }).collect();
    let (status_code, status_msg) = match &s.status {
        Status::Unset => (0, String::new()),
        Status::Ok => (1, String::new()),
        Status::Error(m) => (2, m.clone()),
    };

    json!({
        "traceId": crate::types::format_trace_id(s.context.trace_id),
        "spanId": crate::types::format_span_id(s.context.span_id),
        "parentSpanId": s.parent_span_id.map(crate::types::format_span_id).unwrap_or_default(),
        "name": s.name,
        "kind": kind_int(s.kind),
        "startTimeUnixNano": s.start_time.timestamp_nanos_opt().unwrap_or(0).to_string(),
        "endTimeUnixNano": s.end_time.timestamp_nanos_opt().unwrap_or(0).to_string(),
        "attributes": attributes,
        "events": events,
        "links": links,
        "status": { "code": status_code, "message": status_msg },
    })
}

fn kind_int(k: crate::types::SpanKind) -> i32 {
    match k {
        crate::types::SpanKind::Internal => 1,
        crate::types::SpanKind::Server => 2,
        crate::types::SpanKind::Client => 3,
        crate::types::SpanKind::Producer => 4,
        crate::types::SpanKind::Consumer => 5,
    }
}

#[async_trait]
impl SpanExporter for OtlpHttpExporter {
    async fn export(&self, batch: Vec<SpanData>) -> ExportResult {
        if batch.is_empty() { return Ok(()); }
        let payload = self.render_payload(&batch);
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|e| ExportError::Transport(e.to_string()))?;
        let mut req = client.post(&self.endpoint).json(&payload);
        for (k, v) in &self.headers {
            req = req.header(k, v);
        }
        let resp = req.send().await.map_err(|e| ExportError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(ExportError::Rejected(resp.status().as_u16()));
        }
        Ok(())
    }
}

// ─── TempoExporter ────────────────────────────────────────────────────────

/// Tempo speaks OTLP/HTTP at /api/push (and the standard OTLP path).
/// This is essentially OtlpHttpExporter pinned to Tempo's path with a
/// different default header.
#[derive(Debug, Clone)]
pub struct TempoExporter {
    inner: OtlpHttpExporter,
}

impl TempoExporter {
    pub fn new(base: impl Into<String>) -> Self {
        let endpoint = format!("{}/api/push", base.into().trim_end_matches('/'));
        TempoExporter {
            inner: OtlpHttpExporter::new(endpoint)
                .with_header("Content-Type", "application/x-protobuf"),
        }
    }

    pub fn render_payload(&self, batch: &[SpanData]) -> Value {
        self.inner.render_payload(batch)
    }
}

#[async_trait]
impl SpanExporter for TempoExporter {
    async fn export(&self, batch: Vec<SpanData>) -> ExportResult {
        self.inner.export(batch).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn span(name: &str) -> SpanData {
        let now = Utc::now();
        SpanData {
            name: name.into(),
            context: SpanContext::new(0xdeadbeef_cafe_babe_dead_beef_cafe_babe, 0xfeedface, true),
            parent_span_id: None,
            kind: SpanKind::Server,
            start_time: now,
            end_time: now + chrono::Duration::milliseconds(10),
            attributes: {
                let mut a = HashMap::new();
                a.insert("http.status".into(), AttrValue::Int(200));
                a
            },
            events: vec![Event { name: "ev".into(), time: now, attributes: HashMap::new() }],
            links: vec![],
            status: Status::Ok,
            instrumentation_scope: "test".into(),
            tenant_id: "anonymous".into(),
            resource: HashMap::from([("service.name".into(), "svc".into())]),
        }
    }

    #[tokio::test]
    async fn test_inmemory_collects() {
        let e = InMemoryExporter::new();
        e.export(vec![span("a"), span("b")]).await.unwrap();
        assert_eq!(e.count(), 2);
    }

    #[tokio::test]
    async fn test_inmemory_fail_next_returns_error_then_recovers() {
        let e = InMemoryExporter::new();
        e.fail_next(1);
        assert!(e.export(vec![span("a")]).await.is_err());
        e.export(vec![span("b")]).await.unwrap();
        assert_eq!(e.count(), 1);
    }

    #[tokio::test]
    async fn test_noop_always_ok() {
        let e = NoopExporter;
        assert!(e.export(vec![span("a")]).await.is_ok());
    }

    #[test]
    fn test_otlp_payload_groups_by_scope_and_resource() {
        let exp = OtlpHttpExporter::new("http://localhost");
        let mut s1 = span("a"); s1.instrumentation_scope = "scope-a".into();
        let mut s2 = span("b"); s2.instrumentation_scope = "scope-b".into();
        let v = exp.render_payload(&[s1, s2]);
        let groups = v["resourceSpans"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "two distinct scopes → two resource_spans");
    }

    #[test]
    fn test_otlp_payload_includes_tenant_attribute() {
        let exp = OtlpHttpExporter::new("http://localhost");
        let mut s = span("a"); s.tenant_id = "acme".into();
        let v = exp.render_payload(&[s]);
        let attrs = v["resourceSpans"][0]["resource"]["attributes"].as_array().unwrap();
        let tenant_attr = attrs.iter().find(|a| a["key"] == "tenant_id").unwrap();
        assert_eq!(tenant_attr["value"]["stringValue"], "acme");
    }

    #[test]
    fn test_otlp_payload_emits_span_kind_int() {
        let exp = OtlpHttpExporter::new("http://localhost");
        let s = span("a"); // kind = Server
        let v = exp.render_payload(&[s]);
        let kind = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["kind"];
        assert_eq!(kind, 2);
    }

    #[test]
    fn test_otlp_payload_emits_status_error() {
        let exp = OtlpHttpExporter::new("http://localhost");
        let mut s = span("a");
        s.status = Status::Error("nope".into());
        let v = exp.render_payload(&[s]);
        let status = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["status"];
        assert_eq!(status["code"], 2);
        assert_eq!(status["message"], "nope");
    }

    #[test]
    fn test_otlp_payload_attributes_typed() {
        let exp = OtlpHttpExporter::new("http://localhost");
        let mut s = span("a");
        s.attributes.insert("flag".into(), AttrValue::Bool(true));
        s.attributes.insert("rate".into(), AttrValue::Float(0.42));
        let v = exp.render_payload(&[s]);
        let attrs = v["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"].as_array().unwrap();
        let flag = attrs.iter().find(|a| a["key"] == "flag").unwrap();
        assert_eq!(flag["value"]["boolValue"], true);
        let rate = attrs.iter().find(|a| a["key"] == "rate").unwrap();
        assert!((rate["value"]["doubleValue"].as_f64().unwrap() - 0.42).abs() < 1e-9);
    }

    #[test]
    fn test_otlp_payload_event_serialization() {
        let exp = OtlpHttpExporter::new("http://localhost");
        let s = span("a");
        let v = exp.render_payload(&[s]);
        let ev = &v["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["events"][0];
        assert_eq!(ev["name"], "ev");
    }

    #[test]
    fn test_with_header_appends() {
        let e = OtlpHttpExporter::new("http://x").with_header("X", "1").with_header("Y", "2");
        assert_eq!(e.headers, vec![("X".into(), "1".into()), ("Y".into(), "2".into())]);
    }

    #[test]
    fn test_tempo_endpoint_appends_api_push() {
        let t = TempoExporter::new("http://tempo:3200");
        let v = t.render_payload(&[span("a")]);
        assert!(v["resourceSpans"].is_array());
    }

    #[test]
    fn test_attr_array_values_serialize() {
        let exp = OtlpHttpExporter::new("http://x");
        let mut s = span("a");
        s.attributes.insert("tags".into(), AttrValue::StringArray(vec!["a".into(), "b".into()]));
        s.attributes.insert("nums".into(), AttrValue::IntArray(vec![1, 2, 3]));
        let v = exp.render_payload(&[s]);
        let attrs = v["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"].as_array().unwrap();
        assert!(attrs.iter().any(|a| a["key"] == "tags"));
        assert!(attrs.iter().any(|a| a["key"] == "nums"));
    }

    #[tokio::test]
    async fn test_otlp_exporter_empty_batch_succeeds() {
        let e = OtlpHttpExporter::new("http://0.0.0.0:1");
        assert!(e.export(vec![]).await.is_ok());
    }
}
