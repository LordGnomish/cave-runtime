//! HTTP routes for cave-portal.
//!
//! Serves the embedded SPA at / and all portal API endpoints at /api/v1/portal/*.

use crate::{dashboard, ui, State};
use axum::{
<<<<<<< HEAD
=======
    extract::{Query, State as AxumState},
    http::header,
    response::{Html, IntoResponse, Response},
>>>>>>> claude/determined-visvesvaraya
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        // SPA root — serves the full single-page app
        .route("/", get(serve_spa))
        // Portal API — dashboard aggregation
        .route("/api/v1/portal/dashboard", get(portal_dashboard))
        // Portal API — module listing with health
        .route("/api/v1/portal/modules", get(portal_modules))
        // Portal API — global search
        .route("/api/v1/portal/search", get(portal_search))
        // Portal API — cross-module notification feed
        .route("/api/v1/portal/notifications", get(portal_notifications))
        // Portal API — sidebar navigation structure
        .route("/api/v1/portal/nav", get(portal_nav))
        // Legacy health endpoint (kept for backward compat)
        .route("/api/portal/health", get(health))
        .with_state(state)
}

// ── SPA ───────────────────────────────────────────────────────────

async fn serve_spa() -> Response {
    let html = ui::embedded_ui();
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        Html(html),
    )
        .into_response()
}

// ── Dashboard ─────────────────────────────────────────────────────

async fn portal_dashboard(
    AxumState(_state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let data = dashboard::get_dashboard();
    Json(serde_json::to_value(data).unwrap_or_default())
}

// ── Modules listing ───────────────────────────────────────────────

async fn portal_modules(
    AxumState(_state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let modules = dashboard::list_modules();
    Json(serde_json::to_value(modules).unwrap_or_default())
}

// ── Global search ─────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
}

async fn portal_search(
    AxumState(_state): AxumState<Arc<State>>,
    Query(params): Query<SearchParams>,
) -> Json<serde_json::Value> {
    let query = params.q.unwrap_or_default();
    let results = dashboard::global_search(&query);
    Json(serde_json::to_value(results).unwrap_or_default())
}

// ── Notifications ─────────────────────────────────────────────────

async fn portal_notifications(
    AxumState(_state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let notifs = dashboard::get_notifications();
    Json(serde_json::to_value(notifs).unwrap_or_default())
}

// ── Navigation ────────────────────────────────────────────────────

async fn portal_nav(
    AxumState(_state): AxumState<Arc<State>>,
) -> Json<serde_json::Value> {
    let nav = dashboard::get_nav();
    Json(serde_json::to_value(nav).unwrap_or_default())
}

// ── Health ────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-portal",
        "status": "ok",
        "upstream": "Backstage"
    }))
}
