//! New Relic Traces adapter.
//!
//! Forwards traces to New Relic via the New Relic Trace API (JSON format).
//! Queries use the NerdGraph GraphQL API with NRQL.
//!
//! # Configuration
//!
//! ```toml
//! [trace]
//! backend        = "new_relic"
//! nr_license_key = "..."
//! nr_account_id  = 1234567
//! nr_region      = "US"   # or "EU"
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::backend::{TraceBackend, TraceBackendError, TraceResult};
use crate::types::{Span, SpanStatus, TraceId, format_trace_id, format_span_id};

#[derive(Debug, Clone, Deserialize)]
pub struct NewRelicTraceConfig {
    pub license_key: String,
    /// NR account ID (required for NerdGraph queries).
    pub account_id: Option<u64>,
    /// `"US"` or `"EU"`.
    pub region: String,
}

impl NewRelicTraceConfig {
    fn trace_api_url(&self) -> &'static str {
        match self.region.as_str() {
            "EU" => "https://trace-api.eu.newrelic.com/trace/v1",
            _ => "https://trace-api.newrelic.com/trace/v1",
        }
    }

    fn nerdgraph_url(&self) -> &'static str {
        match self.region.as_str() {
            "EU" => "https://api.eu.newrelic.com/graphql",
            _ => "https://api.newrelic.com/graphql",
        }
    }
}

// ─── New Relic Trace API JSON format ─────────────────────────────────────────
//
// POST /trace/v1 with Content-Type: application/json
// Body: [ { "common": {...}, "spans": [...] } ]

#[derive(Serialize, Clone)]
struct NrTracePayload {
    common: NrCommon,
    spans: Vec<NrSpan>,
}

#[derive(Serialize, Clone)]
struct NrCommon {
    attributes: HashMap<String, serde_json::Value>,
}

#[derive(Serialize, Clone)]
struct NrSpan {
    id: String,
    #[serde(rename = "trace.id")]
    trace_id: String,
    timestamp: i64, // epoch milliseconds
    attributes: HashMap<String, serde_json::Value>,
}

fn build_nr_span(span: &Span) -> NrSpan {
    let mut attrs: HashMap<String, serde_json::Value> = HashMap::new();

    attrs.insert("name".into(), serde_json::Value::String(span.operation_name.clone()));
    attrs.insert(
        "service.name".into(),
        serde_json::Value::String(span.service_name.clone()),
    );
    attrs.insert(
        "duration.ms".into(),
        serde_json::Value::Number(
            serde_json::Number::from_f64(span.duration_ms()).unwrap_or(serde_json::Number::from(0)),
        ),
    );

    if let Some(parent) = span.parent_span_id {
        attrs.insert(
            "parent.id".into(),
            serde_json::Value::String(format_span_id(parent)),
        );
    }

    if span.status == SpanStatus::Error || span.has_error() {
        attrs.insert("error".into(), serde_json::Value::Bool(true));
    }

    // Span kind as string.
    let kind_str = match span.kind {
        crate::types::SpanKind::Server => "server",
        crate::types::SpanKind::Client => "client",
        crate::types::SpanKind::Producer => "producer",
        crate::types::SpanKind::Consumer => "consumer",
        crate::types::SpanKind::Internal => "internal",
    };
    attrs.insert("span.kind".into(), serde_json::Value::String(kind_str.to_string()));

    // Tags / resource attributes.
    for (k, v) in &span.tags {
        attrs.entry(k.clone()).or_insert_with(|| serde_json::Value::String(v.display()));
    }
    for (k, v) in &span.resource_attributes {
        attrs.entry(k.clone()).or_insert_with(|| serde_json::Value::String(v.display()));
    }

    NrSpan {
        id: format_span_id(span.span_id),
        trace_id: format_trace_id(span.trace_id),
        timestamp: (span.start_time_unix_nano / 1_000_000) as i64,
        attributes: attrs,
    }
}

// ─── Adapter ─────────────────────────────────────────────────────────────────

/// New Relic Traces adapter.
pub struct NewRelicTraceAdapter {
    config: NewRelicTraceConfig,
    client: reqwest::Client,
}

impl NewRelicTraceAdapter {
    pub fn new(config: NewRelicTraceConfig) -> Self {
        Self { config, client: reqwest::Client::new() }
    }

    async fn send_payload(&self, payload: Vec<NrTracePayload>) -> TraceResult<()> {
        let resp = self
            .client
            .post(self.config.trace_api_url())
            .header("Api-Key", &self.config.license_key)
            .header("Content-Type", "application/json")
            .header("Data-Format", "newrelic")
            .header("Data-Format-Version", "1")
            .json(&payload)
            .send()
            .await
            .map_err(|e| TraceBackendError::Unreachable(format!("New Relic trace request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(TraceBackendError::IngestFailed(format!(
                "New Relic Trace API returned {status}: {body}"
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl TraceBackend for NewRelicTraceAdapter {
    async fn ingest(&self, spans: Vec<Span>) -> TraceResult<()> {
        if spans.is_empty() {
            return Ok(());
        }

        // Group by trace_id so we can set common.trace.id per batch group.
        let mut by_trace: std::collections::HashMap<u128, Vec<NrSpan>> =
            std::collections::HashMap::new();
        for span in &spans {
            by_trace.entry(span.trace_id).or_default().push(build_nr_span(span));
        }

        // Build one payload entry per trace; chunk at 100 traces per request.
        let payloads: Vec<NrTracePayload> = by_trace
            .into_iter()
            .map(|(trace_id, nr_spans)| {
                let mut common_attrs: HashMap<String, serde_json::Value> = HashMap::new();
                common_attrs.insert(
                    "trace.id".into(),
                    serde_json::Value::String(format_trace_id(trace_id)),
                );
                NrTracePayload {
                    common: NrCommon { attributes: common_attrs },
                    spans: nr_spans,
                }
            })
            .collect();

        // NR Trace API recommends ≤ 5000 spans per batch.
        for chunk in payloads.chunks(100) {
            self.send_payload(chunk.to_vec()).await?;
        }

        Ok(())
    }

    async fn get_trace(&self, trace_id: TraceId) -> TraceResult<Vec<Span>> {
        let account_id = self.config.account_id.ok_or_else(|| {
            TraceBackendError::ConfigError("nr_account_id is required for trace retrieval".into())
        })?;

        let hex = format_trace_id(trace_id);
        let nrql = format!(
            "SELECT * FROM Span WHERE trace.id = '{}' LIMIT MAX SINCE 1 hour ago",
            hex
        );

        let gql = format!(
            r#"{{ "query": "{{ actor {{ account(id: {account_id}) {{ nrql(query: \"{nrql}\") {{ results }} }} }} }}" }}"#,
            account_id = account_id,
            nrql = nrql.replace('"', "\\\""),
        );

        let resp = self
            .client
            .post(self.config.nerdgraph_url())
            .header("Api-Key", &self.config.license_key)
            .header("Content-Type", "application/json")
            .body(gql)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("NR NerdGraph request failed: {e}")))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("NR NerdGraph parse error: {e}")))?;

        // Convert NR span results back to cave Span objects (best-effort).
        let results = json
            .pointer("/data/actor/account/nrql/results")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let spans: Vec<Span> = results
            .into_iter()
            .filter_map(|r| nr_result_to_span(r, trace_id))
            .collect();

        if spans.is_empty() {
            return Err(TraceBackendError::NotFound(format!("{:032x}", trace_id)));
        }

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
        let account_id = self.config.account_id.ok_or_else(|| {
            TraceBackendError::ConfigError("nr_account_id is required for trace search".into())
        })?;

        let mut conditions: Vec<String> = Vec::new();
        if let Some(svc) = service {
            conditions.push(format!("service.name = '{}'", svc.replace('\'', "\\'")));
        }
        if let Some(op) = operation {
            conditions.push(format!("name = '{}'", op.replace('\'', "\\'")));
        }
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {} ", conditions.join(" AND "))
        };

        let since_ms = start_ms;
        let until_ms = end_ms;
        let nrql = format!(
            "SELECT uniques(trace.id, {limit}) FROM Span {where_clause}SINCE {since_ms} milliseconds ago UNTIL {until_ms} milliseconds ago LIMIT 1",
            limit = limit.min(2000),
        );

        let gql = format!(
            r#"{{ "query": "{{ actor {{ account(id: {account_id}) {{ nrql(query: \"{nrql}\") {{ results }} }} }} }}" }}"#,
            account_id = account_id,
            nrql = nrql.replace('"', "\\\""),
        );

        let resp = self
            .client
            .post(self.config.nerdgraph_url())
            .header("Api-Key", &self.config.license_key)
            .header("Content-Type", "application/json")
            .body(gql)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("NR search request failed: {e}")))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("NR search parse error: {e}")))?;

        let ids: Vec<TraceId> = json
            .pointer("/data/actor/account/nrql/results/0/uniques.trace.id")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(|s| u128::from_str_radix(s, 16).ok())
                    .collect()
            })
            .unwrap_or_default();

        Ok(ids)
    }

    async fn services(&self) -> TraceResult<Vec<String>> {
        let Some(account_id) = self.config.account_id else {
            return Ok(vec![]);
        };

        let nrql = "SELECT uniques(service.name, 500) FROM Span SINCE 1 hour ago LIMIT 1";
        let gql = format!(
            r#"{{ "query": "{{ actor {{ account(id: {account_id}) {{ nrql(query: \"{nrql}\") {{ results }} }} }} }}" }}"#,
        );

        let resp = self
            .client
            .post(self.config.nerdgraph_url())
            .header("Api-Key", &self.config.license_key)
            .header("Content-Type", "application/json")
            .body(gql)
            .send()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("NR services request failed: {e}")))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| TraceBackendError::QueryFailed(format!("NR services parse error: {e}")))?;

        let services: Vec<String> = json
            .pointer("/data/actor/account/nrql/results/0/uniques.service.name")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(services)
    }

    fn name(&self) -> &'static str {
        "new-relic-traces"
    }
}

/// Convert a NerdGraph NRQL Span result row to a cave Span (best-effort).
fn nr_result_to_span(row: serde_json::Value, trace_id: TraceId) -> Option<Span> {
    use crate::types::*;

    let span_id_str = row.get("id").or_else(|| row.get("span.id")).and_then(|v| v.as_str())?;
    let span_id = u64::from_str_radix(span_id_str, 16).ok()?;

    let operation_name = row.get("name").and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
    let service_name = row
        .get("service.name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let timestamp_ms = row.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
    let duration_ms = row
        .get("duration.ms")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let start_ns = (timestamp_ms as u64) * 1_000_000;
    let duration_ns = (duration_ms * 1_000_000.0) as u64;

    let parent_span_id = row
        .get("parent.id")
        .and_then(|v| v.as_str())
        .and_then(|s| u64::from_str_radix(s, 16).ok());

    Some(Span {
        trace_id,
        span_id,
        parent_span_id,
        operation_name,
        service_name,
        start_time_unix_nano: start_ns,
        end_time_unix_nano: start_ns + duration_ns,
        duration_ns,
        status: SpanStatus::Unset,
        kind: SpanKind::Internal,
        tags: HashMap::new(),
        events: vec![],
        links: vec![],
        resource_attributes: HashMap::new(),
        tenant_id: String::new(),
        baggage: HashMap::new(),
        log_labels: HashMap::new(),
    })
}
