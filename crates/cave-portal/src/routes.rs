//! HTTP routes for cave-portal.

use crate::State;
use axum::{
    routing::get,
    http::StatusCode,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/portal/health", get(health))
        .route("/api/portal/dashboard", get(dashboard_api))
        .route("/api/portal/nav", get(nav_api))
        .route("/api/portal/modules", get(modules_api))
        .route("/api/portal/search", get(search_api))
        .route("/api/portal/notifications", get(notifications_api))
        .route("/portal/tracker", get(serve_tracker_ui))
        .route("/portal/registry", get(serve_registry_ui))
        .route("/portal/scan", get(serve_scan_ui))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-portal",
        "status": "ok",
        "upstream": "Backstage"
    }))
}

async fn dashboard_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "dashboard": "ok"
    }))
}

async fn nav_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "nav": "ok"
    }))
}

async fn modules_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "modules": "ok"
    }))
}

async fn search_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "search": "ok"
    }))
}

async fn notifications_api() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "notifications": "ok"
    }))
}

async fn serve_tracker_ui() -> (StatusCode, &'static str) {
    (StatusCode::OK, include_str!("tracker_ui.html"))
}

async fn serve_registry_ui() -> (StatusCode, &'static str) {
    (StatusCode::OK, include_str!("registry_ui.html"))
}

async fn serve_scan_ui() -> (StatusCode, &'static str) {
    (StatusCode::OK, include_str!("scan_ui.html"))
}
