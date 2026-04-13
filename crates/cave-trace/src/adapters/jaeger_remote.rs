//! Remote Jaeger collector adapter.
//!
//! Forwards spans to an external Jaeger deployment:
//! - **Ingest**: POST Zipkin v2 JSON to the Jaeger collector's Zipkin-compatible
//!   endpoint (`{collector_url}` should point to port 9411 `/api/v2/spans`, or
//!   the OTLP HTTP endpoint at port 4318 when using Jaeger ≥ 1.35).
//! - **Query**: Jaeger Query HTTP REST API (port 16686 by default).
//!
//! # Configuration
//!
//! ```toml
//! [trace]
//! backend              = "jaeger"
//! jaeger_collector_url = "http://jaeger-collector:9411/api/v2/spans"
//! jaeger_query_url     = "http://jaeger-query:16686"   # optional, enables read
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{TraceBackend, TraceBackendError, TraceResult};
use crate::types::{Span, SpanStatus, TraceId, format_trace_id, format_span_id};

#[derive(Debug, Clone, Deserialize)]
pub struct JaegerRemoteConfig {
    /// Jaeger collector ingest endpoint, e.g.
    /// `http://jaeger-collector:9411/api/v2/spans` (Zipkin v2 JSON).
    pub collector_url: String,
    /// Jaeger Query REST API base URL, e.g. `http://jaeger-query:16686`.
    /// Required for get_trace / search / services.
    pub query_url: Option<String>,
}

// ─── Zipkin v2 JSON format (what Jaeger's Zipkin receiver accepts) ──────────

#[derive(Serialize)]
struct ZipkinSpan {
    id: String,          // 16-char hex span ID
    #[serde(rename = "traceId")]
    trace_id: String,    // 32-char hex trace ID
    #[serde(rename = "parentId", skip_serializing_if = "Option::is_none")]
    parent_id: Option<String>,
    name: String,
    #[serde(rename = "localEndpoint")]
    local_endpoint: ZipkinEndpoint,
    #[serde(rename = "remoteEndpoint", skip_serializing_if = "Option::is_none")]
    remote_endpoint: Option<ZipkinEndpoint>,
    timestamp: i64,      // epoch microseconds
    duration: i64,       // microseconds
    kind: Option<String>,
    tags: HashMap<String, String>,
    annotations: Vec<ZipkinAnnotation>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    debug: bool,
}

#[derive(Serialize)]
struct ZipkinEndpoint {
    #[serde(rename = "serviceName")]
    service_name: String,
}

#[derive(Serialize)]
struct ZipkinAnnotation {
    timestamp: i64, // epoch microseconds
    value: String,
}

fn to_zipkin_span(span: &Span) -> ZipkinSpan {
    let mut tags: HashMap<String, String> = span
        .tags
        .iter()
        .map(|(k, v)| (k.clone(), v.display()))
        .collect();

    // Propagate resource attributes as tags.
    for (k, v) in &span.resource_attributes {
        tags.entry(k.clone()).or_insert_with(|| v.display());
    }

    if span.status == SpanStatus::Error || span.has_error() {
        tags.insert("error".into(), "true".into());
    }

    let kind = match span.kind {
        crate::types::SpanKind::Server => Some("SERVER"),
        crate::types::SpanKind::Client => Some("CLIENT"),
        crate::types::SpanKind::Producer => Some("PRODUCER"),
        crate::types::SpanKind::Consumer => Some("CONSUMER"),
        crate::types::SpanKind::Internal => None,
    };

    let remote_endpoint = tags
        .get("peer.service")
        .map(|svc| ZipkinEndpoint { service_name: svc.clone() });

    let annotations = span
        .events
        .iter()
        .map(|e| ZipkinAnnotation {
            timestamp: (e.time_unix_nano / 1000) as i64,
            value: e.name.clone(),
        })
        .collect();

    ZipkinSpan {
        id: format_span_id(span.span_id),
        trace_id: format_trace_id(span.trace_id),
        parent_id: span.parent_span_id.map(format_span_id),
        name: span.operation_name.clone(),
        local_endpoint: ZipkinEndpoint { service_name: span.service_name.clone() },
        remote_endpoint,
        timestamp: (span.start_time_unix_nano / 1000) as i64,
        duration: (span.duration_ns / 1000) as i64,
        kind: kind.map(|s| s.to_string()),
        tags,
        annotations,
        debug: false,
    }
}

// ─── Jaeger Query REST API response types ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JaegerQueryResponse<T> {
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct JaegerTraceData {
    #[serde(rename = "traceID")]
    trace_id: String,
    spans: Vec<JaegerSpanData>,
    processes: HashMap<String, JaegerProcess>,
}

#[derive(Debug, Deserialize)]
struct JaegerSpanData {
    #[serde(rename = "traceID")]
    trace_id: String,
    #[serde(rename = "spanID")]
    span_id: String,
    #[serde(rename = "operationName")]
    operation_name: String,
    #[serde(rename = "startTime")]
    start_time: i64, // epoch microseconds
    duration: i64,   // microseconds
    #[serde(rename = "processID")]
    process_id: String,
    tags: Vec<JaegerTag>,
    logs: Vec<JaegerLog>,
    references: Vec<JaegerReference>,
}

#[derive(Debug, Deserialize)]
struct JaegerProcess {
    #[serde(rename = "serviceName")]
    service_name: String,
}

#[derive(Debug, Deserialize)]
struct JaegerTag {
    key: String,
    #[serde(rename = "type")]
    tag_type: String,
    value: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JaegerLog {
    timestamp: i64,
    fields: Vec<JaegerTag>,
}

#[derive(Debug, Deserialize)]
struct JaegerReference {
    #[serde(rename = "refType")]
    ref_type: String,
    #[serde(rename = "traceID")]
    trace_id: String,
    #[serde(rename = "spanID")]
    span_id: String,
}

fn jaeger_tag_to_string(tag: &JaegerTag) -> String {
    match tag.value {
        serde_json::Value::String(ref s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(ref n) => n.to_string(),
        _ => tag.value.to_string(),
    }
}

fn jaeger_span_to_cave(jspan: &JaegerSpanData, processes: &HashMap<String, JaegerProcess>) -> Span {
    use crate::types::*;

    let service_name = processes
        .get(&jspan.process_id)
        .map(|p| p.service_name.clone())
        .unwrap_or_default();

    let tags: HashMap<String, TagValue> = jspan
        .tags
        .iter()
        .map(|t| (t.key.clone(), TagValue::String(jaeger_tag_to_string(t))))
        .collect();

    let parent_span_id = jspan.references.iter().find_map(|r| {
        if r.ref_type == "CHILD_OF" {
            u64::from_str_radix(&r.span_id, 16).ok()
        } else {
            None
        }
    });

    let trace_id = u128::from_str_radix(&jspan.trace_id, 16).unwrap_or(0);
    let span_id = u64::from_str_radix(&jspan.span_id, 16).unwrap_or(0);
    let start_ns = (jspan.start_time as u64) * 1000;
    let duration_ns = (jspan.duration as u64) * 1000;

    let events: Vec<SpanEvent> = jspan
        .logs
        .iter()
        .map(|log| SpanEvent {
            time_unix_nano: (log.timestamp as u64) * 1000,
            name: log
                .fields
                .iter()
                .find(|f| f.key == "event")
                .map(|f| jaeger_tag_to_string(f))
                .unwrap_or_else(|| "log".to_string()),
            attributes: log
                .fields
                .iter()
                .map(|f| (f.key.clone(), TagValue::String(jaeger_tag_to_string(f))))
                .collect(),
        })
        .collect();

    Span {
        trace_id,
        span_id,
        parent_span_id,
        operation_name: jspan.operation_name.clone(),
        service_name,
        start_time_unix_nano: start_ns,
        end_time_unix_nano: start_ns + duration_ns,
        duration_ns,
        status: SpanStatus::Unset,
        kind: SpanKind::Internal,
        tags,
        events,
        links: vec![],
        resource_attributes: HashMap::new(),
        tenant_id: String::new(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    }
}

// ─── Adapter ─────────────────────────────────────────────────────────────────

/// Remote Jaeger collector adapter.
pub struct JaegerRemoteAdapter {
    config: JaegerRemoteConfig,
    client: reqwest::Client,
}

impl JaegerRemoteAdapter {
    pub fn new(config: JaegerRemoteConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    fn query_url(&self) -> TraceResult<&str> {
        self.config.query_url.as_deref().ok_or_else(|| {
            TraceBackendError::ConfigError(
                "jaeger_query_url must be set to enable trace retrieval".into(),
            )
        })
    }
}

#[async_trait]
impl TraceBackend for JaegerRemoteAdapter {
    async fn ingest(&self, spans: Vec<Span>) -> TraceResult<()> {
        if spans.is_empty() {
            return Ok(());
        }

        let zipkin_spans: Vec<ZipkinSpan> = spans.iter().map(to_zipkin_span).collect();

        let resp = self
            .client
            .post(&self.config.collector_url)
            .header("Content-Type", "application/json")
            .json(&zipkin_spans)
            .send()
            .await
            .map_err(|e| TraceBackendError::Unreachable(format!("Jaeger collector request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TraceBackendError::IngestFailed(format!(
                "Jaeger collector returned {status}: {body}"
            )));
        }

        Ok(())
    }

    async fn get_trace(&self, trace_id: TraceId) -> TraceResult<Vec<Span>> {
        let base = self.query_url()?;
        let hex = format_trace_id(trace_id);
        let url = format!("{}/api/traces/{}", base.trim_end_matches('/'), hex);

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Jaeger get trace failed: {e}")))?;

        if resp.status() == 404 {
            return Err(TraceBackendError::NotFound(format!("{:032x}", trace_id)));
        }

        let jqr: JaegerQueryResponse<JaegerTraceData> = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Jaeger response parse error: {e}")))?;

        let spans: Vec<Span> = jqr
            .data
            .into_iter()
            .flat_map(|trace| {
                let processes = trace.processes;
                trace
                    .spans
                    .iter()
                    .map(|s| jaeger_span_to_cave(s, &processes))
                    .collect::<Vec<_>>()
            })
            .collect();

        Ok(spans)
    }

    async fn search(
        &self,
        service: Option<&str>,
        operation: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> TraceResult<Vec<TraceId>> {
        let base = self.query_url()?;
        let mut url = format!(
            "{}/api/traces?start={}&end={}&limit={}",
            base.trim_end_matches('/'),
            start_ms * 1000,   // ms → µs
            end_ms * 1000,
            limit.min(1000),
        );
        if let Some(svc) = service {
            url.push_str(&format!("&service={}", urlencoding(svc)));
        }
        if let Some(op) = operation {
            url.push_str(&format!("&operation={}", urlencoding(op)));
        }

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Jaeger search failed: {e}")))?;

        let jqr: JaegerQueryResponse<JaegerTraceData> = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Jaeger search parse error: {e}")))?;

        let ids: Vec<TraceId> = jqr
            .data
            .iter()
            .filter_map(|t| u128::from_str_radix(&t.trace_id, 16).ok())
            .collect();

        Ok(ids)
    }

    async fn services(&self) -> TraceResult<Vec<String>> {
        let base = self.query_url()?;
        let url = format!("{}/api/services", base.trim_end_matches('/'));

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Jaeger services failed: {e}")))?;

        let jqr: JaegerQueryResponse<String> = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Jaeger services parse error: {e}")))?;

        Ok(jqr.data)
    }

    fn name(&self) -> &'static str {
        "jaeger-remote"
    }
}

/// Minimal percent-encoding for query parameter values (replaces space and reserved chars).
fn urlencoding(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                vec![c]
            }
            c => format!("%{:02X}", c as u32).chars().collect::<Vec<_>>(),
        })
        .collect()
}
