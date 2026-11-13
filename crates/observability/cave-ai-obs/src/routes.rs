// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-ai-obs — Langfuse-compatible LLM observability API.

use crate::State;
use crate::analytics::{compute_aggregated_stats, compute_cost_window, top_models_by_cost};
use crate::trace_models::{Generation, Score, Span, Trace, TraceStatus};
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    routing::{get, post},
};
use chrono::Duration;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        // Health
        .route("/api/ai-obs/health", get(health))
        // Trace ingestion
        .route("/api/ai-obs/traces", post(ingest_trace))
        .route("/api/ai-obs/traces", get(list_traces))
        .route("/api/ai-obs/traces/{id}", get(get_trace))
        // Span ingestion
        .route("/api/ai-obs/spans", post(ingest_span))
        .route("/api/ai-obs/traces/{id}/spans", get(get_spans))
        // Generation ingestion
        .route("/api/ai-obs/generations", post(ingest_generation))
        .route("/api/ai-obs/traces/{id}/generations", get(get_generations))
        // Scores
        .route("/api/ai-obs/scores", post(ingest_score))
        .route("/api/ai-obs/traces/{id}/scores", get(get_scores))
        // Analytics
        .route("/api/ai-obs/stats", get(get_stats))
        .route("/api/ai-obs/stats/costs", get(get_cost_stats))
        .route("/api/ai-obs/stats/models", get(get_model_stats))
        // Prompt management
        .route("/api/ai-obs/prompts", post(upsert_prompt))
        .route("/api/ai-obs/prompts/{name}", get(get_prompt))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-ai-obs",
        "status": "ok",
        "upstream": "Langfuse",
        "version": "v3.75.1"
    }))
}

// ─── Trace Ingestion ─────────────────────────────────────────────────────

async fn ingest_trace(
    AxumState(state): AxumState<Arc<State>>,
    Json(trace): Json<Trace>,
) -> (StatusCode, Json<serde_json::Value>) {
    state.store.upsert_trace(trace.clone());
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": trace.id,
            "status": "created"
        })),
    )
}

#[derive(Deserialize)]
struct TraceListQuery {
    user_id: Option<String>,
    session_id: Option<String>,
    tag: Option<String>,
    limit: Option<usize>,
}

async fn list_traces(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<TraceListQuery>,
) -> Json<Vec<Trace>> {
    let limit = q.limit.unwrap_or(50).min(1000);
    Json(state.store.list_traces(
        q.user_id.as_deref(),
        q.session_id.as_deref(),
        q.tag.as_deref(),
        limit,
    ))
}

async fn get_trace(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Trace>, StatusCode> {
    state
        .store
        .get_trace(&id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

// ─── Span Ingestion ───────────────────────────────────────────────────────

async fn ingest_span(
    AxumState(state): AxumState<Arc<State>>,
    Json(span): Json<Span>,
) -> (StatusCode, Json<serde_json::Value>) {
    let id = span.id;
    state.store.upsert_span(span);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "status": "created" })),
    )
}

async fn get_spans(
    AxumState(state): AxumState<Arc<State>>,
    Path(trace_id): Path<Uuid>,
) -> Json<Vec<Span>> {
    Json(state.store.get_spans_for_trace(&trace_id))
}

// ─── Generation Ingestion ─────────────────────────────────────────────────

async fn ingest_generation(
    AxumState(state): AxumState<Arc<State>>,
    Json(generation): Json<Generation>,
) -> (StatusCode, Json<serde_json::Value>) {
    let id = generation.id;
    state.store.upsert_generation(generation);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "status": "created" })),
    )
}

async fn get_generations(
    AxumState(state): AxumState<Arc<State>>,
    Path(trace_id): Path<Uuid>,
) -> Json<Vec<Generation>> {
    Json(state.store.get_generations_for_trace(&trace_id))
}

// ─── Score Ingestion ──────────────────────────────────────────────────────

async fn ingest_score(
    AxumState(state): AxumState<Arc<State>>,
    Json(score): Json<Score>,
) -> (StatusCode, Json<serde_json::Value>) {
    let id = score.id;
    state.store.upsert_score(score);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "status": "created" })),
    )
}

async fn get_scores(
    AxumState(state): AxumState<Arc<State>>,
    Path(trace_id): Path<Uuid>,
) -> Json<Vec<Score>> {
    Json(state.store.get_scores_for_trace(&trace_id))
}

// ─── Analytics ────────────────────────────────────────────────────────────

async fn get_stats(AxumState(state): AxumState<Arc<State>>) -> Json<serde_json::Value> {
    let stats = compute_aggregated_stats(&state.store);
    Json(serde_json::to_value(stats).unwrap_or(serde_json::Value::Null))
}

#[derive(Deserialize)]
struct CostWindowQuery {
    /// Window in hours (default: 24)
    hours: Option<i64>,
}

async fn get_cost_stats(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<CostWindowQuery>,
) -> Json<serde_json::Value> {
    let hours = q.hours.unwrap_or(24);
    let window = compute_cost_window(&state.store, Duration::hours(hours));
    Json(serde_json::to_value(window).unwrap_or(serde_json::Value::Null))
}

#[derive(Deserialize)]
struct TopModelsQuery {
    limit: Option<usize>,
}

async fn get_model_stats(
    AxumState(state): AxumState<Arc<State>>,
    Query(q): Query<TopModelsQuery>,
) -> Json<serde_json::Value> {
    let limit = q.limit.unwrap_or(10).min(100);
    let top = top_models_by_cost(&state.store, limit);
    Json(serde_json::to_value(top).unwrap_or(serde_json::Value::Null))
}

// ─── Prompt Management ────────────────────────────────────────────────────

async fn upsert_prompt(
    AxumState(state): AxumState<Arc<State>>,
    Json(tmpl): Json<crate::trace_models::PromptTemplate>,
) -> (StatusCode, Json<serde_json::Value>) {
    let name = tmpl.name.clone();
    let version = tmpl.version;
    state.store.upsert_prompt_template(tmpl);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({ "name": name, "version": version, "status": "created" })),
    )
}

#[derive(Deserialize)]
struct PromptQuery {
    version: Option<u32>,
}

async fn get_prompt(
    AxumState(state): AxumState<Arc<State>>,
    Path(name): Path<String>,
    Query(q): Query<PromptQuery>,
) -> Result<Json<crate::trace_models::PromptTemplate>, StatusCode> {
    let tmpl = if let Some(v) = q.version {
        state.store.get_prompt_template(&name, v)
    } else {
        state.store.get_active_prompt(&name)
    };
    tmpl.map(Json).ok_or(StatusCode::NOT_FOUND)
}
