//! HTTP routes for cave-gateway (Kong + Gravitee unified).
//!
//! Kong routes:   /api/gateway/...
//! Gravitee routes: /api/v1/gateway/...

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<GatewayState>) -> Router {
    // Kong-side routes, plus Gravitee module routers merged before the final .with_state().
    Router::new()
        // ── Kong routes ───────────────────────────────────────────────────────
        .route("/api/gateway/health", get(health))
        .route("/api/gateway/metrics", get(metrics))
        .route("/api/gateway/check", post(check_request))
        .route("/api/gateway/routes", get(list_routes).post(create_route))
        .route("/api/gateway/routes/:id", get(get_route).delete(delete_route))
        .route("/api/gateway/upstreams", get(list_upstreams).post(create_upstream))
        .route("/api/gateway/upstreams/:id", get(get_upstream).delete(delete_upstream))
        .route("/api/gateway/upstreams/:id/health", post(trigger_health_check))
        .route("/api/gateway/circuit-breakers", get(circuit_breaker_status))
        .route("/api/gateway/upstreams/:upstream_id/result", post(record_result))
        // ── Gravitee module routes ────────────────────────────────────────────
        .merge(crate::api_designer::routes())
        .merge(crate::marketplace::routes())
        .merge(crate::monetization::routes())
        .merge(crate::lifecycle::routes())
        .merge(crate::protocols::routes())
        .merge(crate::flows::routes())
        // ── Unified state ─────────────────────────────────────────────────────
        .with_state(state)
}

// ── Kong handlers ─────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-gateway",
        "status": "ok",
        "kong_compatible": true,
        "gravitee_extensions": [
            "api-designer", "quality-scoring", "marketplace",
            "monetization", "lifecycle", "protocols", "flows"
        ]
    }))
}

async fn metrics(State(state): State<Arc<GatewayState>>) -> Json<GatewayMetrics> {
    let engine = state.engine.lock().unwrap();
    Json(engine.metrics.clone())
}

async fn check_request(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CheckRequest>,
) -> Json<CheckResponse> {
    let mut engine = state.engine.lock().unwrap();
    let result = engine.evaluate_request(
        &req.path,
        &req.method,
        &req.client_ip,
        req.auth_header.as_deref(),
        req.user_agent.as_deref(),
        req.body_size.unwrap_or(0),
    );
    Json(result)
}

async fn list_routes(State(state): State<Arc<GatewayState>>) -> Json<Vec<Route>> {
    let engine = state.engine.lock().unwrap();
    Json(engine.routes.clone())
}

async fn create_route(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateRouteRequest>,
) -> Json<Route> {
    let mut engine = state.engine.lock().unwrap();
    Json(engine.add_route(req))
}

async fn get_route(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let engine = state.engine.lock().unwrap();
    match engine.routes.iter().find(|r| r.id == id) {
        Some(route) => Json(serde_json::to_value(route).unwrap()),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn delete_route(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut engine = state.engine.lock().unwrap();
    Json(serde_json::json!({ "removed": engine.remove_route(id) }))
}

async fn list_upstreams(State(state): State<Arc<GatewayState>>) -> Json<Vec<UpstreamService>> {
    let engine = state.engine.lock().unwrap();
    Json(engine.upstreams.clone())
}

async fn create_upstream(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateUpstreamRequest>,
) -> Json<UpstreamService> {
    let mut engine = state.engine.lock().unwrap();
    Json(engine.add_upstream(req))
}

async fn get_upstream(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let engine = state.engine.lock().unwrap();
    match engine.upstreams.iter().find(|u| u.id == id) {
        Some(u) => Json(serde_json::to_value(u).unwrap()),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

async fn delete_upstream(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut engine = state.engine.lock().unwrap();
    Json(serde_json::json!({ "removed": engine.remove_upstream(id) }))
}

async fn circuit_breaker_status(State(state): State<Arc<GatewayState>>) -> Json<Vec<CircuitBreakerStatus>> {
    let engine = state.engine.lock().unwrap();
    Json(engine.circuit_breaker_statuses())
}

#[derive(serde::Deserialize)]
struct RecordResultRequest {
    success: bool,
}

async fn record_result(
    State(state): State<Arc<GatewayState>>,
    Path(upstream_id): Path<Uuid>,
    Json(req): Json<RecordResultRequest>,
) -> Json<serde_json::Value> {
    let mut engine = state.engine.lock().unwrap();
    engine.record_upstream_result(upstream_id, req.success);
    Json(serde_json::json!({ "recorded": true }))
}

/// Trigger health checks against all nodes of an upstream.
/// Lock is dropped before HTTP calls to avoid holding it across await points.
async fn trigger_health_check(
    State(state): State<Arc<GatewayState>>,
    Path(upstream_id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let (nodes, health_path) = {
        let engine = state.engine.lock().unwrap();
        let upstream = match engine.upstreams.iter().find(|u| u.id == upstream_id) {
            Some(u) => u,
            None => return Json(serde_json::json!({ "error": "upstream not found" })),
        };
        let path = upstream.health_check.as_ref()
            .map(|hc| hc.path.clone())
            .unwrap_or_else(|| "/health".to_string());
        (upstream.nodes.clone(), path)
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let mut results = Vec::new();
    for node in &nodes {
        let url = format!("http://{}{}", node.address, health_path);
        let healthy = client.get(&url).send().await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        results.push((node.id, healthy));
    }

    let mut engine = state.engine.lock().unwrap();
    for (node_id, healthy) in &results {
        engine.set_node_health(upstream_id, *node_id, *healthy);
    }

    Json(serde_json::json!({
        "upstream_id": upstream_id,
        "checked": results.len(),
        "results": results.iter().map(|(id, h)| serde_json::json!({
            "node_id": id, "healthy": h
        })).collect::<Vec<_>>()
    }))
}
