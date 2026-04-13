//! Datadog APM adapter.
//!
//! Forwards traces to Datadog APM via the Datadog Agent (port 8126) or
//! directly to `trace.agent.{site}` using the v0.4 JSON trace format.
//!
//! # Configuration
//!
//! ```toml
//! [trace]
//! backend      = "datadog"
//! dd_api_key   = "..."
//! dd_site      = "datadoghq.com"
//! dd_agent_url = "http://datadog-agent:8126"   # preferred for low latency
//! dd_service   = "cave-runtime"
//! dd_env       = "production"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{TraceBackend, TraceBackendError, TraceResult};
use crate::types::{Span, SpanStatus, TraceId, format_trace_id, format_span_id};

#[derive(Debug, Clone, Deserialize)]
pub struct DatadogApmConfig {
    pub api_key: String,
    pub site: String,
    /// Datadog Agent endpoint (preferred). If set, POSTs there without API key.
    pub agent_url: Option<String>,
    pub service: String,
    pub env: String,
}

impl DatadogApmConfig {
    /// Ingest URL: agent if configured, otherwise the Datadog intake API.
    fn ingest_url(&self) -> String {
        if let Some(ref url) = self.agent_url {
            format!("{}/v0.4/traces", url.trim_end_matches('/'))
        } else {
            format!("https://trace.agent.{}/api/v0.2/traces", self.site)
        }
    }

    fn search_url(&self) -> String {
        format!("https://api.{}/api/v2/spans/events/search", self.site)
    }
}

// ─── Datadog v0.4 span wire format ─────────────────────────────────────────

/// Datadog span in the v0.4 JSON trace format.
#[derive(Serialize, Clone)]
struct DdSpan {
    trace_id: u64,   // lower 64 bits of trace_id (Datadog uses u64)
    span_id: u64,
    parent_id: u64,  // 0 if root
    name: String,
    resource: String,
    service: String,
    #[serde(rename = "type")]
    span_type: String,
    start: i64,      // epoch nanoseconds (i64 for Datadog)
    duration: i64,   // nanoseconds
    error: i32,      // 0 or 1
    meta: HashMap<String, String>,
    metrics: HashMap<String, f64>,
}

fn span_type_for(span: &Span) -> String {
    if let Some(v) = span.tags.get("db.type").or_else(|| span.tags.get("db.system")) {
        return format!("db:{}", v.display());
    }
    if span.tags.contains_key("http.method") || span.tags.contains_key("http.url") {
        return "web".into();
    }
    if span.tags.contains_key("messaging.system") {
        return "queue".into();
    }
    "custom".into()
}

fn to_dd_span(span: &Span, default_service: &str, env: &str) -> DdSpan {
    let mut meta: HashMap<String, String> = span
        .tags
        .iter()
        .map(|(k, v)| (k.clone(), v.display()))
        .collect();

    meta.insert("env".into(), env.to_string());

    // Carry resource attributes into meta.
    for (k, v) in &span.resource_attributes {
        meta.entry(k.clone()).or_insert_with(|| v.display());
    }

    let mut metrics: HashMap<String, f64> = HashMap::new();
    // Promote numeric tags to metrics (e.g. http.status_code).
    for (k, v) in &span.tags {
        if let Some(f) = v.as_f64() {
            metrics.insert(k.clone(), f);
        }
    }

    let resource = span
        .tags
        .get("http.route")
        .or_else(|| span.tags.get("http.url"))
        .or_else(|| span.tags.get("db.statement"))
        .map(|v| v.display())
        .unwrap_or_else(|| span.operation_name.clone());

    DdSpan {
        // Datadog uses 64-bit trace IDs; take the lower 64 bits.
        trace_id: span.trace_id as u64,
        span_id: span.span_id,
        parent_id: span.parent_span_id.unwrap_or(0),
        name: span.operation_name.clone(),
        resource,
        service: if span.service_name.is_empty() {
            default_service.to_string()
        } else {
            span.service_name.clone()
        },
        span_type: span_type_for(span),
        start: span.start_time_unix_nano as i64,
        duration: span.duration_ns as i64,
        error: if span.status == SpanStatus::Error || span.has_error() { 1 } else { 0 },
        meta,
        metrics,
    }
}

// ─── Adapter ─────────────────────────────────────────────────────────────────

/// Datadog APM adapter.
pub struct DatadogApmAdapter {
    config: DatadogApmConfig,
    client: reqwest::Client,
}

impl DatadogApmAdapter {
    pub fn new(config: DatadogApmConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    async fn send_traces(&self, traces: Vec<Vec<DdSpan>>) -> TraceResult<()> {
        let body = serde_json::to_string(&traces)
            .map_err(|e| TraceBackendError::IngestFailed(format!("Serialization error: {e}")))?;

        let mut req = self
            .client
            .post(self.config.ingest_url())
            .header("Content-Type", "application/json")
            .header("X-Datadog-Trace-Count", traces.len().to_string());

        // Only send API key when posting directly (not via agent).
        if self.config.agent_url.is_none() {
            req = req.header("DD-API-KEY", &self.config.api_key);
        }

        let resp = req
            .body(body)
            .send()
            .await
            .map_err(|e| TraceBackendError::Unreachable(format!("Datadog APM request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(TraceBackendError::IngestFailed(format!(
                "Datadog APM returned {status}: {body_text}"
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl TraceBackend for DatadogApmAdapter {
    async fn ingest(&self, spans: Vec<Span>) -> TraceResult<()> {
        if spans.is_empty() {
            return Ok(());
        }

        // Group spans by trace_id → each group becomes one DD trace (inner Vec).
        let mut by_trace: std::collections::HashMap<u128, Vec<DdSpan>> =
            std::collections::HashMap::new();
        for span in &spans {
            let dd = to_dd_span(span, &self.config.service, &self.config.env);
            by_trace.entry(span.trace_id).or_default().push(dd);
        }

        let traces: Vec<Vec<DdSpan>> = by_trace.into_values().collect();

        // Datadog recommends ≤ 1000 spans per request; chunk if needed.
        for chunk in traces.chunks(50) {
            self.send_traces(chunk.to_vec()).await?;
        }

        Ok(())
    }

    async fn get_trace(&self, _trace_id: TraceId) -> TraceResult<Vec<Span>> {
        // Datadog does not expose a public trace-by-ID retrieval API.
        Err(TraceBackendError::QueryFailed(
            "DatadogApmAdapter: trace retrieval not supported via public API".to_string(),
        ))
    }

    async fn search(
        &self,
        service: Option<&str>,
        operation: Option<&str>,
        start_ms: i64,
        end_ms: i64,
        limit: usize,
    ) -> TraceResult<Vec<TraceId>> {
        // Datadog Spans Search API (v2).
        let mut filter_parts: Vec<String> = Vec::new();
        if let Some(svc) = service {
            filter_parts.push(format!("service:{}", svc));
        }
        if let Some(op) = operation {
            filter_parts.push(format!("operation_name:{}", op));
        }
        let query = if filter_parts.is_empty() {
            "*".to_string()
        } else {
            filter_parts.join(" ")
        };

        let from = chrono::DateTime::from_timestamp(start_ms / 1000, 0)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();
        let to = chrono::DateTime::from_timestamp(end_ms / 1000, 0)
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339();

        let payload = serde_json::json!({
            "filter": { "from": from, "to": to, "query": query },
            "page": { "limit": limit.min(1000) },
            "sort": "timestamp",
        });

        let resp = self
            .client
            .post(self.config.search_url())
            .header("DD-API-KEY", &self.config.api_key)
            .header(
                "DD-APPLICATION-KEY",
                std::env::var("DD_APP_KEY").unwrap_or_default(),
            )
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Datadog spans search failed: {e}")))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("Datadog search parse error: {e}")))?;

        // Extract unique trace IDs from response data array.
        let ids: Vec<TraceId> = json
            .get("data")
            .and_then(|d| d.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.pointer("/attributes/trace_id")
                            .and_then(|v| v.as_str())
                            .and_then(|s| u128::from_str_radix(s, 16).ok())
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ids)
    }

    async fn services(&self) -> TraceResult<Vec<String>> {
        // No simple public services list API in Datadog — return empty.
        Ok(vec![])
    }

    fn name(&self) -> &'static str {
        "datadog-apm"
    }
}
