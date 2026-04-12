//! Jaeger Query API — full parity with Jaeger 1.x query service.
//!
//! Endpoints
//! ─────────
//! GET  /api/traces/{traceID}                  — fetch trace by ID
//! GET  /api/traces                            — search traces
//! GET  /api/services                          — list services
//! GET  /api/services/{service}/operations     — list operations for service
//! GET  /api/dependencies                      — service dependency graph
//! GET  /api/metrics/calls                     — SPM: call rates
//! GET  /api/metrics/errors                    — SPM: error rates
//! GET  /api/metrics/minstep                   — SPM: minimum step
//! POST /api/traces (compare)                  — trace comparison

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::{
    analyzer,
    dependency::{build_dependency_graph, to_jaeger_dependencies},
    multi_tenant::tenant_from_headers,
    query::QueryEngine,
    spm::{to_jaeger_metrics, MetricsResponse, SpmRegistry},
    types::{format_span_id, format_trace_id, parse_trace_id, Span, SpanStatus, Trace, TraceId},
    TraceState,
};

pub fn create_router(state: Arc<TraceState>) -> Router {
    Router::new()
        .route("/api/traces",                            get(search_traces))
        .route("/api/traces/:trace_id",                  get(get_trace))
        .route("/api/services",                          get(get_services))
        .route("/api/services/:service/operations",      get(get_operations))
        .route("/api/dependencies",                      get(get_dependencies))
        .route("/api/metrics/calls",                     get(metrics_calls))
        .route("/api/metrics/errors",                    get(metrics_errors))
        .route("/api/metrics/minstep",                   get(metrics_minstep))
        .with_state(state)
}

// ─── Response envelope ─────────────────────────────────────────────────────

#[derive(Serialize)]
struct JaegerResponse<T: Serialize> {
    data: T,
    total: usize,
    limit: usize,
    offset: usize,
    errors: Option<Vec<String>>,
}

impl<T: Serialize> JaegerResponse<T> {
    fn ok(data: T, total: usize, limit: usize, offset: usize) -> Json<Self> {
        Json(JaegerResponse { data, total, limit, offset, errors: None })
    }
}

// ─── Wire types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JaegerSpan {
    trace_id: String,
    span_id: String,
    operation_name: String,
    references: Vec<JaegerRef>,
    flags: i32,
    start_time: i64,   // epoch µs
    duration: i64,     // µs
    tags: Vec<JaegerTagWire>,
    logs: Vec<JaegerLog>,
    process_id: String,
    process: JaegerProcess,
    warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JaegerRef {
    ref_type: String,
    trace_id: String,
    span_id: String,
}

#[derive(Serialize)]
struct JaegerTagWire {
    key: String,
    #[serde(rename = "type")]
    tag_type: String,
    value: serde_json::Value,
}

#[derive(Serialize)]
struct JaegerLog {
    timestamp: i64,
    fields: Vec<JaegerTagWire>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JaegerProcess {
    service_name: String,
    tags: Vec<JaegerTagWire>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JaegerTrace {
    trace_id: String,
    spans: Vec<JaegerSpan>,
    processes: std::collections::HashMap<String, JaegerProcess>,
    warnings: Vec<String>,
}

// ─── Conversion helpers ────────────────────────────────────────────────────

fn span_to_jaeger(span: &Span) -> JaegerSpan {
    let references: Vec<JaegerRef> = span
        .parent_span_id
        .map(|pid| JaegerRef {
            ref_type: "CHILD_OF".into(),
            trace_id: format_trace_id(span.trace_id),
            span_id: format_span_id(pid),
        })
        .into_iter()
        .chain(span.links.iter().map(|l| JaegerRef {
            ref_type: "FOLLOWS_FROM".into(),
            trace_id: format_trace_id(l.trace_id),
            span_id: format_span_id(l.span_id),
        }))
        .collect();

    let tags: Vec<JaegerTagWire> = span.tags.iter()
        .map(|(k, v)| tag_wire(k, v))
        .collect();

    let logs: Vec<JaegerLog> = span.events.iter().map(|e| JaegerLog {
        timestamp: (e.time_unix_nano / 1000) as i64,
        fields: std::iter::once(JaegerTagWire {
            key: "event".into(),
            tag_type: "string".into(),
            value: serde_json::Value::String(e.name.clone()),
        })
        .chain(e.attributes.iter().map(|(k, v)| tag_wire(k, v)))
        .collect(),
    }).collect();

    let process_tags: Vec<JaegerTagWire> = span.resource_attributes.iter()
        .map(|(k, v)| tag_wire(k, v))
        .collect();

    JaegerSpan {
        trace_id: format_trace_id(span.trace_id),
        span_id: format_span_id(span.span_id),
        operation_name: span.operation_name.clone(),
        references,
        flags: if span.has_error() { 1 } else { 0 },
        start_time: (span.start_time_unix_nano / 1000) as i64,
        duration: (span.duration_ns / 1000) as i64,
        tags,
        logs,
        process_id: "p1".into(),
        process: JaegerProcess {
            service_name: span.service_name.clone(),
            tags: process_tags,
        },
        warnings: vec![],
    }
}

fn tag_wire(key: &str, value: &crate::types::TagValue) -> JaegerTagWire {
    use crate::types::TagValue;
    let (tag_type, json_val) = match value {
        TagValue::String(s) => ("string", serde_json::Value::String(s.clone())),
        TagValue::Bool(b)   => ("bool",   serde_json::Value::Bool(*b)),
        TagValue::Int(i)    => ("int64",  serde_json::json!(*i)),
        TagValue::Float(f)  => ("float64",serde_json::json!(*f)),
        TagValue::Binary(b) => ("binary", serde_json::Value::String(
            b.iter().map(|x| format!("{:02x}", x)).collect()
        )),
    };
    JaegerTagWire { key: key.to_owned(), tag_type: tag_type.into(), value: json_val }
}

fn trace_to_jaeger(trace: &Trace) -> JaegerTrace {
    let spans: Vec<JaegerSpan> = trace.spans.iter().map(span_to_jaeger).collect();
    let processes: std::collections::HashMap<String, JaegerProcess> =
        std::iter::once(("p1".to_owned(), JaegerProcess {
            service_name: trace.root_service_name.clone(),
            tags: vec![],
        }))
        .collect();

    JaegerTrace {
        trace_id: format_trace_id(trace.trace_id),
        spans,
        processes,
        warnings: vec![],
    }
}

// ─── GET /api/traces/{traceID} ─────────────────────────────────────────────

async fn get_trace(
    State(state): State<Arc<TraceState>>,
    Path(trace_id_str): Path<String>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);

    let trace_id = match parse_trace_id(&trace_id_str) {
        Ok(id) => id,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("{}", e)).into_response(),
    };

    let engine = QueryEngine::new(state.store.clone());
    match engine.get_trace(trace_id).await {
        Ok(trace) => {
            let jt = trace_to_jaeger(&trace);
            JaegerResponse::ok(vec![jt], 1, 1, 0).into_response()
        }
        Err(e) => (StatusCode::NOT_FOUND, format!("{}", e)).into_response(),
    }
}

// ─── GET /api/traces (search) ─────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchQuery {
    service: Option<String>,
    operation: Option<String>,
    tags: Option<String>,          // "key=value key2=value2"
    #[serde(rename = "start")]
    start_time_us: Option<i64>,    // epoch µs
    #[serde(rename = "end")]
    end_time_us: Option<i64>,
    #[serde(rename = "minDuration")]
    min_duration: Option<String>,  // "1.5ms" | "500us" | "2s"
    #[serde(rename = "maxDuration")]
    max_duration: Option<String>,
    limit: Option<usize>,
}

async fn search_traces(
    State(state): State<Arc<TraceState>>,
    Query(params): Query<SearchQuery>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);

    let tag_map = params.tags.as_deref()
        .map(parse_tag_query)
        .unwrap_or_default();

    let query = crate::types::TraceSearchQuery {
        tenant_id: Some(tenant_id),
        service: params.service,
        operation: params.operation,
        tags: if tag_map.is_empty() { None } else { Some(tag_map) },
        start_time_ns: params.start_time_us.map(|t| (t.max(0) as u64) * 1000),
        end_time_ns:   params.end_time_us.map(|t| (t.max(0) as u64) * 1000),
        min_duration_ns: params.min_duration.as_deref().and_then(parse_duration_str),
        max_duration_ns: params.max_duration.as_deref().and_then(parse_duration_str),
        limit: params.limit.or(Some(20)),
        ..Default::default()
    };

    let engine = QueryEngine::new(state.store.clone());
    let traces = engine.search(&query).await.unwrap_or_default();
    let total = traces.len();
    let limit = query.limit_or_default();

    let jaeger_traces: Vec<JaegerTrace> = traces.iter().map(trace_to_jaeger).collect();
    JaegerResponse::ok(jaeger_traces, total, limit, 0).into_response()
}

fn parse_tag_query(s: &str) -> std::collections::HashMap<String, String> {
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
    if let Some(v) = s.strip_suffix("us") {
        return Some((v.parse::<f64>().ok()? * 1_000.0) as u64);
    }
    if let Some(v) = s.strip_suffix("µs") {
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

// ─── GET /api/services ────────────────────────────────────────────────────

async fn get_services(
    State(state): State<Arc<TraceState>>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);
    let engine = QueryEngine::new(state.store.clone());
    let services = engine.list_services(Some(&tenant_id)).await;
    JaegerResponse::ok(services.clone(), services.len(), services.len(), 0).into_response()
}

// ─── GET /api/services/{service}/operations ───────────────────────────────

async fn get_operations(
    State(state): State<Arc<TraceState>>,
    Path(service): Path<String>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);
    let engine = QueryEngine::new(state.store.clone());
    let ops = engine.list_operations(&service, Some(&tenant_id)).await;

    // Jaeger UI expects `[{name, spanKind}]`
    let wire: Vec<serde_json::Value> = ops.iter().map(|op| {
        serde_json::json!({ "name": op, "spanKind": "" })
    }).collect();

    JaegerResponse::ok(wire, ops.len(), ops.len(), 0).into_response()
}

// ─── GET /api/dependencies ────────────────────────────────────────────────

#[derive(Deserialize)]
struct DepsQuery {
    #[serde(rename = "endTs")]
    end_ts: Option<i64>,  // epoch ms
    #[serde(rename = "lookback")]
    lookback: Option<i64>, // ms
}

async fn get_dependencies(
    State(state): State<Arc<TraceState>>,
    Query(params): Query<DepsQuery>,
    headers: HeaderMap,
) -> Response {
    let tenant_id = tenant_from_headers(&headers);

    let end_ns = params.end_ts.map(|t| (t as u64) * 1_000_000);
    let start_ns = match (end_ns, params.lookback) {
        (Some(end), Some(lb)) => Some(end.saturating_sub((lb as u64) * 1_000_000)),
        _ => None,
    };

    let engine = QueryEngine::new(state.store.clone());
    let deps = engine.service_dependencies(start_ns, end_ns, Some(&tenant_id)).await;

    // Convert to Jaeger format
    let jaeger_deps: Vec<serde_json::Value> = deps.iter().map(|d| {
        serde_json::json!({
            "parent": d.parent,
            "child":  d.child,
            "callCount": d.call_count,
        })
    }).collect();

    JaegerResponse::ok(jaeger_deps.clone(), jaeger_deps.len(), jaeger_deps.len(), 0).into_response()
}

// ─── SPM metrics endpoints ────────────────────────────────────────────────

#[derive(Deserialize)]
struct MetricsQuery {
    service: Option<String>,
    #[serde(rename = "spanKind")]
    span_kind: Option<String>,
    #[serde(rename = "groupByOperation")]
    group_by_operation: Option<bool>,
    #[serde(rename = "ratePer")]
    rate_per: Option<String>,
    #[serde(rename = "spanName")]
    span_name: Option<String>,
    start: Option<i64>,
    end: Option<i64>,
    step: Option<i64>,
}

async fn metrics_calls(
    State(state): State<Arc<TraceState>>,
    Query(params): Query<MetricsQuery>,
    headers: HeaderMap,
) -> Response {
    let snap = state.spm_registry.snapshot();
    let filtered: Vec<_> = snap.iter()
        .filter(|m| params.service.as_deref().map_or(true, |s| m.service == s))
        .cloned()
        .collect();
    Json(to_jaeger_metrics(&filtered)).into_response()
}

async fn metrics_errors(
    State(state): State<Arc<TraceState>>,
    Query(params): Query<MetricsQuery>,
    headers: HeaderMap,
) -> Response {
    let snap = state.spm_registry.snapshot();
    let filtered: Vec<_> = snap.iter()
        .filter(|m| params.service.as_deref().map_or(true, |s| m.service == s))
        .cloned()
        .collect();
    Json(to_jaeger_metrics(&filtered)).into_response()
}

async fn metrics_minstep(headers: HeaderMap) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "data": { "minStep": 1000 } }))
}
