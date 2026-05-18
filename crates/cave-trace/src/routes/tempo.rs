// SPDX-License-Identifier: AGPL-3.0-or-later
//! Grafana Tempo HTTP API — full parity with Tempo 2.x.
//!
//! Endpoints
//! ─────────
//! GET  /api/traces/{traceID}           — fetch trace by ID (Tempo format)
//! GET  /api/search                     — search traces (tags + duration + time range)
//! GET  /api/search/tags                — list searchable tag names
//! GET  /api/search/tag/{name}/values   — list values for a tag
//! GET  /api/echo                       — health / connectivity check
//! GET  /api/status/buildinfo           — build info
//! POST /api/search                     — TraceQL search (body = query string)

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    multi_tenant::tenant_from_headers,
    query::QueryEngine,
    spm::SpmRegistry,
    traceql,
    types::{
        format_span_id, format_trace_id, parse_trace_id, Span, SpanKind, SpanStatus, TagValue,
        Trace, TraceId, TraceSearchQuery,
    },
    TraceState,
};

pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        .route("/tempo/api/traces/{trace_id}",        get(get_trace))
        .route("/tempo/api/search",                  get(search_traces).post(search_traceql))
        .route("/tempo/api/search/tags",             get(search_tags))
        .route("/tempo/api/search/tag/{name}/values", get(search_tag_values))
        .route("/tempo/api/echo",                    get(echo))
        .route("/tempo/api/status/buildinfo",        get(build_info))
        .with_state(state)
}

// ─── Tempo trace wire format ───────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TempoTrace {
    trace_id: String,
    root_service_name: String,
    root_trace_name: String,
    start_time_unix_nano: String, // u64 as string
    duration_ms: f64,
    span_set: Option<SpanSet>,
    spans: Vec<TempoSpan>,
}

#[derive(Serialize)]
struct SpanSet {
    spans: Vec<TempoSpan>,
    matched: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TempoSpan {
    span_id: String,
    operation: String,
    start_time_unix_nano: String,
    duration_nanos: String,
    service: TempoService,
    attributes: Vec<TempoAttr>,
    status: String,
    kind: String,
}

#[derive(Serialize)]
struct TempoService {
    name: String,
}

#[derive(Serialize)]
struct TempoAttr {
    key: String,
    value: TempoValue,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TempoValue {
    string_value: Option<String>,
    int_value: Option<String>,
    double_value: Option<f64>,
    bool_value: Option<bool>,
}

fn tag_to_tempo_value(v: &TagValue) -> TempoValue {
    match v {
        TagValue::String(s) => TempoValue { string_value: Some(s.clone()), int_value: None, double_value: None, bool_value: None },
        TagValue::Int(i)    => TempoValue { string_value: None, int_value: Some(i.to_string()), double_value: None, bool_value: None },
        TagValue::Float(f)  => TempoValue { string_value: None, int_value: None, double_value: Some(*f), bool_value: None },
        TagValue::Bool(b)   => TempoValue { string_value: None, int_value: None, double_value: None, bool_value: Some(*b) },
        TagValue::Binary(b) => TempoValue { string_value: Some(b.iter().map(|x| format!("{:02x}", x)).collect()), int_value: None, double_value: None, bool_value: None },
    }
}

fn span_to_tempo(span: &Span) -> TempoSpan {
    let attrs: Vec<TempoAttr> = span.tags.iter()
        .map(|(k, v)| TempoAttr { key: k.clone(), value: tag_to_tempo_value(v) })
        .collect();

    let status = match span.status {
        SpanStatus::Ok    => "STATUS_CODE_OK",
        SpanStatus::Error => "STATUS_CODE_ERROR",
        SpanStatus::Unset => "STATUS_CODE_UNSET",
    };
    let kind = match span.kind {
        SpanKind::Server   => "SPAN_KIND_SERVER",
        SpanKind::Client   => "SPAN_KIND_CLIENT",
        SpanKind::Producer => "SPAN_KIND_PRODUCER",
        SpanKind::Consumer => "SPAN_KIND_CONSUMER",
        SpanKind::Internal => "SPAN_KIND_INTERNAL",
    };

    TempoSpan {
        span_id: format_span_id(span.span_id),
        operation: span.operation_name.clone(),
        start_time_unix_nano: span.start_time_unix_nano.to_string(),
        duration_nanos: span.duration_ns.to_string(),
        service: TempoService { name: span.service_name.clone() },
        attributes: attrs,
        status: status.into(),
        kind: kind.into(),
    }
}

fn trace_to_tempo(trace: &Trace) -> TempoTrace {
    TempoTrace {
        trace_id: format_trace_id(trace.trace_id),
        root_service_name: trace.root_service_name.clone(),
        root_trace_name: trace.root_operation_name.clone(),
        start_time_unix_nano: trace.start_time_unix_nano.to_string(),
        duration_ms: trace.duration_ns as f64 / 1_000_000.0,
        span_set: Some(SpanSet {
            spans: trace.spans.iter().map(span_to_tempo).collect(),
            matched: trace.spans.len(),
        }),
        spans: trace.spans.iter().map(span_to_tempo).collect(),
    }
}

// ─── GET /api/traces/{traceID} ────────────────────────────────────────────

async fn get_trace(
    State(state): State<Arc<TraceState>>,
    Path(trace_id_str): Path<String>,
    headers: HeaderMap,
) -> Response {
    let trace_id = match parse_trace_id(&trace_id_str) {
        Ok(id) => id,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{}", e)).into_response(),
    };

    let engine = QueryEngine::new(state.store.clone());
    match engine.get_trace(trace_id).await {
        Ok(trace) => Json(serde_json::json!({
            "trace": trace_to_tempo(&trace)
        })).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

// ─── GET /api/search ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TempoSearchQuery {
    q: Option<String>,           // TraceQL query
    tags: Option<String>,        // "key=value" space-separated
    #[serde(rename = "minDuration")]
    min_duration: Option<String>,
    #[serde(rename = "maxDuration")]
    max_duration: Option<String>,
    start: Option<i64>,          // epoch seconds
    end: Option<i64>,
    limit: Option<usize>,
    offset: Option<usize>,
    #[serde(rename = "spss")]
    spss: Option<usize>,         // spans per span set
}

async fn search_traces(
    State(state): State<Arc<TraceState>>,
    Query(params): Query<TempoSearchQuery>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);
    let engine = QueryEngine::new(state.store.clone());

    // If there's a TraceQL query, use TraceQL engine
    if let Some(ref q) = params.q {
        return search_with_traceql(q, &state, &engine, &tenant_id, &params).await;
    }

    let tag_map: std::collections::HashMap<String, String> = params
        .tags
        .as_deref()
        .map(|t| parse_tag_pairs(t))
        .unwrap_or_default();

    let query = TraceSearchQuery {
        tenant_id: Some(tenant_id),
        tags: if tag_map.is_empty() { None } else { Some(tag_map) },
        min_duration_ns: params.min_duration.as_deref().and_then(parse_duration_str),
        max_duration_ns: params.max_duration.as_deref().and_then(parse_duration_str),
        start_time_ns: params.start.map(|s| s as u64 * 1_000_000_000),
        end_time_ns:   params.end.map(|e| e as u64 * 1_000_000_000),
        limit: params.limit.or(Some(20)),
        offset: params.offset,
        ..Default::default()
    };

    let traces = engine.search(&query).await.unwrap_or_default();

    let results: Vec<serde_json::Value> = traces.iter().map(|t| {
        serde_json::json!({
            "traceID": format_trace_id(t.trace_id),
            "rootServiceName": t.root_service_name,
            "rootTraceName": t.root_operation_name,
            "startTimeUnixNano": t.start_time_unix_nano.to_string(),
            "durationMs": t.duration_ns as f64 / 1_000_000.0,
            "spanSets": [{
                "spans": t.spans.iter().map(span_to_tempo).collect::<Vec<_>>(),
                "matched": t.spans.len(),
            }]
        })
    }).collect();

    Json(serde_json::json!({ "traces": results, "metrics": { "inspectedTraces": traces.len() } }))
        .into_response()
}

async fn search_with_traceql(
    query: &str,
    state: &TraceState,
    engine: &QueryEngine,
    tenant_id: &str,
    params: &TempoSearchQuery,
) -> Response {
    let pred = match traceql::parse(query) {
        Ok(p) => p,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("TraceQL error: {}", e)).into_response(),
    };

    // Fetch a broad set of spans and filter with TraceQL
    let broad_query = TraceSearchQuery {
        tenant_id: Some(tenant_id.to_owned()),
        start_time_ns: params.start.map(|s| s as u64 * 1_000_000_000),
        end_time_ns:   params.end.map(|e| e as u64 * 1_000_000_000),
        limit: Some(1000), // fetch more to allow TraceQL post-filtering
        ..Default::default()
    };

    let traces = engine.search(&broad_query).await.unwrap_or_default();
    let limit = params.limit.unwrap_or(20);

    let matching: Vec<serde_json::Value> = traces.iter()
        .filter(|t| t.spans.iter().any(|s| traceql::eval_span(&pred, s)))
        .take(limit)
        .map(|t| {
            let matched_spans: Vec<TempoSpan> = t.spans.iter()
                .filter(|s| traceql::eval_span(&pred, s))
                .map(span_to_tempo)
                .collect();
            serde_json::json!({
                "traceID": format_trace_id(t.trace_id),
                "rootServiceName": t.root_service_name,
                "rootTraceName": t.root_operation_name,
                "startTimeUnixNano": t.start_time_unix_nano.to_string(),
                "durationMs": t.duration_ns as f64 / 1_000_000.0,
                "spanSets": [{
                    "spans": matched_spans,
                    "matched": matched_spans.len(),
                }]
            })
        })
        .collect();

    Json(serde_json::json!({ "traces": matching })).into_response()
}

// ─── POST /api/search (TraceQL body) ─────────────────────────────────────

async fn search_traceql(
    State(state): State<Arc<TraceState>>,
    Query(params): Query<TempoSearchQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);
    let engine = QueryEngine::new(state.store.clone());

    // Body may be a JSON object with "q" field or a raw TraceQL string
    let query_str = if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&body) {
        json.get("q")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .unwrap_or_default()
    } else {
        String::from_utf8(body.to_vec()).unwrap_or_default()
    };

    if query_str.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing TraceQL query").into_response();
    }

    search_with_traceql(&query_str, &state, &engine, &tenant_id, &params).await
}

// ─── GET /api/search/tags ─────────────────────────────────────────────────

async fn search_tags(
    State(state): State<Arc<TraceState>>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);
    let engine = QueryEngine::new(state.store.clone());
    let tags = engine.list_tag_names(Some(&tenant_id)).await;

    // Tempo returns both span-scoped and resource-scoped tags
    let mut all_tags = tags.clone();
    // Add well-known resource tag names
    for t in ["service.name", "deployment.environment", "k8s.namespace.name", "k8s.pod.name"] {
        if !all_tags.contains(&t.to_owned()) {
            all_tags.push(t.to_owned());
        }
    }
    all_tags.sort();
    all_tags.dedup();

    Json(serde_json::json!({
        "tagNames": all_tags,
        "scopes": [
            { "name": "span",     "tags": tags },
            { "name": "resource", "tags": all_tags },
        ]
    })).into_response()
}

// ─── GET /api/search/tag/{name}/values ───────────────────────────────────

async fn search_tag_values(
    State(state): State<Arc<TraceState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);
    let engine = QueryEngine::new(state.store.clone());
    let values = engine.list_tag_values(&name, Some(&tenant_id)).await;
    Json(serde_json::json!({ "tagValues": values })).into_response()
}

// ─── Utility endpoints ────────────────────────────────────────────────────

async fn echo() -> Response {
    "echo".into_response()
}

async fn build_info() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": "cave-trace/0.1.0",
        "revision": env!("CARGO_PKG_VERSION"),
        "branch": "main",
        "buildDate": "unknown",
        "goVersion": "n/a",
        "featureFlags": []
    }))
}

// ─── Helpers ──────────────────────────────────────────────────────────────

fn parse_tag_pairs(s: &str) -> std::collections::HashMap<String, String> {
    s.split_whitespace()
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

fn parse_duration_str(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(v) = s.strip_suffix("ms") {
        return Some((v.parse::<f64>().ok()? * 1_000_000.0) as u64);
    }
    if let Some(v) = s.strip_suffix("us").or_else(|| s.strip_suffix("µs")) {
        return Some((v.parse::<f64>().ok()? * 1_000.0) as u64);
    }
    if let Some(v) = s.strip_suffix('s') {
        return Some((v.parse::<f64>().ok()? * 1_000_000_000.0) as u64);
    }
    if let Some(v) = s.strip_suffix('m') {
        return Some((v.parse::<f64>().ok()? * 60_000_000_000.0) as u64);
    }
    None
}
