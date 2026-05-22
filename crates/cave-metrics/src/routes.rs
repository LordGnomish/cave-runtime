// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-metrics.
//!
//! Exposes two route groups:
//!   /api/metrics/*  — cave-native management API
//!   /api/v1/*       — Prometheus-compatible HTTP API (drop-in replacement)
//!   /metrics        — Prometheus exposition format (self-metrics)

use crate::{
    models::*,
    MetricsState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::{collections::HashMap, sync::Arc};

pub fn create_router(state: Arc<MetricsState>) -> Router {
    Router::new()
        // ── cave-native ────────────────────────────────────────────────────
        .route("/api/metrics/health", get(health))
        // ── Prometheus remote_write ────────────────────────────────────────
        // POST /api/v1/write — accepts snappy-compressed protobuf body
        // (Prometheus remote_write protocol v1 / v2)
        .route("/api/v1/write", post(remote_write))
        // ── Prometheus query API ───────────────────────────────────────────
        // Instant query:  GET /api/v1/query?query=<promql>&time=<ts>
        .route("/api/v1/query", get(instant_query))
        // Range query:    GET /api/v1/query_range?query=<promql>&start=<ts>&end=<ts>&step=<dur>
        .route("/api/v1/query_range", get(range_query))
        // ── Prometheus metadata API ────────────────────────────────────────
        // Series:         GET /api/v1/series?match[]=<selector>
        .route("/api/v1/series", get(series))
        // Label names:    GET /api/v1/labels
        .route("/api/v1/labels", get(labels))
        // Label values:   GET /api/v1/label/{name}/values
        .route("/api/v1/label/{name}/values", get(label_values))
        // ── Self-metrics (Prometheus exposition format) ────────────────────
        .route("/api/metrics/self", get(self_metrics))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// cave-native endpoints
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-metrics",
        "status": "ok",
        "upstream": "prometheus",
        "upstream_tracked_version": "2.x",
        "compat": ["remote_write_v1", "http_api_v1"]
    }))
}

// ---------------------------------------------------------------------------
// Prometheus remote_write — POST /api/v1/write
// ---------------------------------------------------------------------------

/// Accept Prometheus remote_write payload (snappy-encoded protobuf).
///
/// Real implementation will decode the WriteRequest protobuf and persist
/// samples to the TSDB. For now we accept the bytes and return 204, which
/// is the correct response code per the remote_write spec.
async fn remote_write(
    State(_state): State<Arc<MetricsState>>,
    body: Bytes,
) -> StatusCode {
    // TODO: decode snappy + protobuf WriteRequest, ingest into TSDB
    tracing::debug!(bytes = body.len(), "remote_write received");
    StatusCode::NO_CONTENT
}

// ---------------------------------------------------------------------------
// Prometheus instant query — GET /api/v1/query
// ---------------------------------------------------------------------------

async fn instant_query(
    State(_state): State<Arc<MetricsState>>,
    Query(params): Query<InstantQueryParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(query = %params.query, "instant_query");
    // TODO: evaluate PromQL against TSDB
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "vector",
            "result": []
        }
    }))
}

// ---------------------------------------------------------------------------
// Prometheus range query — GET /api/v1/query_range
// ---------------------------------------------------------------------------

async fn range_query(
    State(_state): State<Arc<MetricsState>>,
    Query(params): Query<RangeQueryParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(
        query = %params.query,
        start = %params.start,
        end   = %params.end,
        step  = %params.step,
        "range_query"
    );
    // TODO: evaluate PromQL range query against TSDB
    Json(serde_json::json!({
        "status": "success",
        "data": {
            "resultType": "matrix",
            "result": []
        }
    }))
}

// ---------------------------------------------------------------------------
// Series metadata — GET /api/v1/series
// ---------------------------------------------------------------------------

async fn series(
    State(_state): State<Arc<MetricsState>>,
    Query(_params): Query<SeriesParams>,
) -> Json<serde_json::Value> {
    // TODO: query TSDB for matching series
    let result: Vec<HashMap<String, String>> = vec![];
    Json(serde_json::json!({
        "status": "success",
        "data": result
    }))
}

// ---------------------------------------------------------------------------
// Label names — GET /api/v1/labels
// ---------------------------------------------------------------------------

async fn labels(
    State(_state): State<Arc<MetricsState>>,
    Query(_params): Query<LabelsParams>,
) -> Json<serde_json::Value> {
    // TODO: enumerate all label names from TSDB
    let data: Vec<String> = vec!["__name__".to_string(), "job".to_string(), "instance".to_string()];
    Json(serde_json::json!({
        "status": "success",
        "data": data
    }))
}

// ---------------------------------------------------------------------------
// Label values — GET /api/v1/label/{name}/values
// ---------------------------------------------------------------------------

async fn label_values(
    State(_state): State<Arc<MetricsState>>,
    Path(name): Path<String>,
    Query(_params): Query<LabelValuesParams>,
) -> Json<serde_json::Value> {
    tracing::debug!(label = %name, "label_values");
    // TODO: query TSDB for distinct values of this label
    let data: Vec<String> = vec![];
    Json(serde_json::json!({
        "status": "success",
        "data": data
    }))
}

// ---------------------------------------------------------------------------
// Self-metrics — GET /metrics  (Prometheus exposition format)
// ---------------------------------------------------------------------------

/// Return cave-metrics own operational metrics in Prometheus text format.
/// Content-Type must be text/plain; version=0.0.4 per the exposition spec.
async fn self_metrics(State(_state): State<Arc<MetricsState>>) -> (StatusCode, [(& 'static str, &'static str); 1], String) {
    // TODO: expose real counters/gauges via prometheus-client crate
    let body = "# HELP cave_metrics_requests_total Total requests processed\n\
                # TYPE cave_metrics_requests_total counter\n\
                cave_metrics_requests_total{handler=\"remote_write\"} 0\n\
                cave_metrics_requests_total{handler=\"query\"} 0\n\
                # HELP cave_metrics_series_total Total time series stored\n\
                # TYPE cave_metrics_series_total gauge\n\
                cave_metrics_series_total 0\n"
        .to_string();
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}
    alerting::{evaluate_alert_rules, firing_alerts, group_alerts},
    query::{execute_query, execute_range_query},
    scraper::service_discovery,
    storage::insert_samples,
    extract::{Path, Query, State as AxumState},
    routing::{delete, get, post, put},
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;
        // Health
        .route("/api/v1/metrics/health", get(health))
        // Write
        .route("/api/v1/metrics/write", post(write_metrics))
        // Query
        .route("/api/v1/metrics/query", get(instant_query))
        .route("/api/v1/metrics/query_range", get(range_query))
        // Series / labels
        .route("/api/v1/metrics/series", get(list_series))
        .route("/api/v1/metrics/labels", get(list_labels))
        // Alert rules CRUD
        .route("/api/v1/metrics/alerts", get(list_alert_rules))
        .route("/api/v1/metrics/alerts", post(create_alert_rule))
        .route("/api/v1/metrics/alerts/{id}", get(get_alert_rule))
        .route("/api/v1/metrics/alerts/{id}", put(update_alert_rule))
        .route("/api/v1/metrics/alerts/{id}", delete(delete_alert_rule))
        .route("/api/v1/metrics/alerts/evaluate", post(trigger_alert_evaluation))
        .route("/api/v1/metrics/alerts/firing", get(list_firing_alerts))
        // Recording rules CRUD
        .route("/api/v1/metrics/rules", get(list_recording_rules))
        .route("/api/v1/metrics/rules", post(create_recording_rule))
        .route("/api/v1/metrics/rules/{id}", delete(delete_recording_rule))
        // Scrape targets CRUD
        .route("/api/v1/metrics/targets", get(list_targets))
        .route("/api/v1/metrics/targets", post(create_target))
        .route("/api/v1/metrics/targets/{id}", delete(delete_target))
        .route("/api/v1/metrics/targets/discover", post(discover_targets))
    Json(json!({
        "upstream": "Prometheus + Thanos"
// ---- Write ----
async fn write_metrics(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Json(req): Json<WriteRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let mut store = state.store.lock().await;
    let samples: Vec<crate::models::Sample> = req
        .samples
        .into_iter()
        .map(|s| crate::models::Sample {
            timestamp: s.timestamp.unwrap_or_else(Utc::now),
            value: s.value,
        })
        .collect();
    let count = samples.len();
    insert_samples(&mut store, &req.metric_name, &req.labels, samples);
        StatusCode::NO_CONTENT,
        Json(json!({ "written": count })),
// ---- Query ----
#[derive(Deserialize)]
struct InstantQueryParams {
    query: String,
    time: Option<String>,
    AxumState(state): AxumState<Arc<MetricsState>>,
) -> Json<QueryResult> {
    let at = params
        .time
        .as_deref()
        .and_then(|t| t.parse::<chrono::DateTime<Utc>>().ok())
        .unwrap_or_else(Utc::now);
    let store = state.store.lock().await;
    Json(execute_query(&store, &params.query, at))
#[derive(Deserialize)]
struct RangeQueryParams {
    query: String,
    start: String,
    end: String,
    step: Option<u64>,
    AxumState(state): AxumState<Arc<MetricsState>>,
) -> Result<Json<QueryResult>, (StatusCode, Json<serde_json::Value>)> {
    let start = params.start.parse::<chrono::DateTime<Utc>>().map_err(|_| {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid start" })))
    })?;
    let end = params.end.parse::<chrono::DateTime<Utc>>().map_err(|_| {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid end" })))
    })?;
    let step = params.step.unwrap_or(60) as i64;
    let store = state.store.lock().await;
    Ok(Json(execute_range_query(&store, &params.query, start, end, step)))
// ---- Series / Labels ----
#[derive(Deserialize)]
struct SeriesParams {
    #[serde(rename = "match[]")]
    match_expr: Option<String>,
async fn list_series(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Query(params): Query<SeriesParams>,
    let store = state.store.lock().await;
    let filter = params.match_expr.as_deref().unwrap_or("");
    let series: Vec<serde_json::Value> = store
        .series
        .values()
        .filter(|ts| filter.is_empty() || ts.metric_name.contains(filter))
        .map(|ts| {
            let mut labels = ts.labels.clone();
            labels.insert("__name__".to_string(), ts.metric_name.clone());
            json!(labels)
        })
        .collect();
    Json(json!({ "status": "success", "data": series }))
async fn list_labels(
    AxumState(state): AxumState<Arc<MetricsState>>,
    let store = state.store.lock().await;
    let mut labels: std::collections::HashSet<String> = std::collections::HashSet::new();
    labels.insert("__name__".to_string());
    for ts in store.series.values() {
        for k in ts.labels.keys() {
            labels.insert(k.clone());
    let mut sorted: Vec<_> = labels.into_iter().collect();
    sorted.sort();
    Json(json!({ "status": "success", "data": sorted }))
// ---- Alert Rules ----
async fn list_alert_rules(
    AxumState(state): AxumState<Arc<MetricsState>>,
    let rules = state.alert_rules.lock().await;
    let groups = group_alerts(&rules);
    Json(json!({ "status": "success", "data": groups.keys().collect::<Vec<_>>(), "rules": *rules }))
async fn get_alert_rule(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Path(id): Path<String>,
) -> Json<Option<AlertRule>> {
    let rules = state.alert_rules.lock().await;
    let uid = Uuid::parse_str(&id).unwrap_or_default();
    Json(rules.iter().find(|r| r.id == uid).cloned())
async fn create_alert_rule(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Json(req): Json<CreateAlertRuleRequest>,
) -> (StatusCode, Json<AlertRule>) {
    let mut rule = AlertRule::new(&req.name, &req.group, &req.expr);
    if let Some(d) = req.for_duration_seconds {
        rule.for_duration_seconds = d;
    if let Some(labels) = req.labels {
        rule.labels = labels;
    if let Some(annotations) = req.annotations {
        rule.annotations = annotations;
    let mut rules = state.alert_rules.lock().await;
    rules.push(rule.clone());
    (StatusCode::CREATED, Json(rule))
async fn update_alert_rule(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateAlertRuleRequest>,
) -> Json<Option<AlertRule>> {
    let uid = Uuid::parse_str(&id).unwrap_or_default();
    let mut rules = state.alert_rules.lock().await;
    if let Some(rule) = rules.iter_mut().find(|r| r.id == uid) {
        rule.name = req.name;
        rule.group = req.group;
        rule.expr = req.expr;
        if let Some(d) = req.for_duration_seconds {
            rule.for_duration_seconds = d;
        if let Some(labels) = req.labels {
            rule.labels = labels;
        if let Some(annotations) = req.annotations {
            rule.annotations = annotations;
        return Json(Some(rule.clone()));
    Json(None)
async fn delete_alert_rule(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Path(id): Path<String>,
    let uid = Uuid::parse_str(&id).unwrap_or_default();
    let mut rules = state.alert_rules.lock().await;
    rules.retain(|r| r.id != uid);
    Json(json!({ "deleted": id }))
async fn trigger_alert_evaluation(
    AxumState(state): AxumState<Arc<MetricsState>>,
    let store = state.store.lock().await;
    let mut rules = state.alert_rules.lock().await;
    evaluate_alert_rules(&mut rules, &store);
    let firing = firing_alerts(&rules).len();
    Json(json!({ "evaluated": rules.len(), "firing": firing }))
async fn list_firing_alerts(
    AxumState(state): AxumState<Arc<MetricsState>>,
    let rules = state.alert_rules.lock().await;
    let firing: Vec<_> = firing_alerts(&rules).into_iter().cloned().collect();
    Json(json!({ "status": "success", "data": firing }))
// ---- Recording Rules ----
async fn list_recording_rules(
    AxumState(state): AxumState<Arc<MetricsState>>,
) -> Json<Vec<RecordingRule>> {
    Json(state.recording_rules.lock().await.clone())
async fn create_recording_rule(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Json(req): Json<CreateRecordingRuleRequest>,
) -> (StatusCode, Json<RecordingRule>) {
    let mut rule = RecordingRule::new(&req.name, &req.group, &req.expr);
    if let Some(interval) = req.interval_seconds {
        rule.interval_seconds = interval;
    if let Some(labels) = req.labels {
        rule.labels = labels;
    let mut rules = state.recording_rules.lock().await;
    rules.push(rule.clone());
    (StatusCode::CREATED, Json(rule))
async fn delete_recording_rule(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Path(id): Path<String>,
    let uid = Uuid::parse_str(&id).unwrap_or_default();
    let mut rules = state.recording_rules.lock().await;
    rules.retain(|r| r.id != uid);
    Json(json!({ "deleted": id }))
// ---- Scrape Targets ----
async fn list_targets(
    AxumState(state): AxumState<Arc<MetricsState>>,
) -> Json<Vec<ScrapeTarget>> {
    Json(state.scrape_targets.lock().await.clone())
async fn create_target(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Json(req): Json<CreateScrapeTargetRequest>,
) -> (StatusCode, Json<ScrapeTarget>) {
    let mut target = ScrapeTarget::new(&req.job, &req.address);
    if let Some(path) = req.metrics_path {
        target.metrics_path = path;
    if let Some(scheme) = req.scheme {
        target.scheme = scheme;
    if let Some(interval) = req.scrape_interval_seconds {
        target.scrape_interval_seconds = interval;
    if let Some(timeout) = req.scrape_timeout_seconds {
        target.scrape_timeout_seconds = timeout;
    if let Some(labels) = req.labels {
        target.labels = labels;
    let mut targets = state.scrape_targets.lock().await;
    targets.push(target.clone());
    (StatusCode::CREATED, Json(target))
async fn delete_target(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Path(id): Path<String>,
    let uid = Uuid::parse_str(&id).unwrap_or_default();
    let mut targets = state.scrape_targets.lock().await;
    targets.retain(|t| t.id != uid);
    Json(json!({ "deleted": id }))
#[derive(Deserialize)]
struct DiscoverRequest {
    job: String,
    addresses: Vec<String>,
async fn discover_targets(
    AxumState(state): AxumState<Arc<MetricsState>>,
    Json(req): Json<DiscoverRequest>,
) -> Json<Vec<ScrapeTarget>> {
    let addrs: Vec<&str> = req.addresses.iter().map(|s| s.as_str()).collect();
    let discovered = service_discovery(&req.job, &addrs);
    let mut targets = state.scrape_targets.lock().await;
    for t in &discovered {
        targets.push(t.clone());
    Json(discovered)
