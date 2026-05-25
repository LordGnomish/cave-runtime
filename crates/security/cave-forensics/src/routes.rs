// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-forensics.

use crate::State;
use axum::{Json, Router, extract, routing::get};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/forensics/health", get(health))
        .route("/api/forensics/cases", get(list_cases))
        .route("/api/forensics/policies", get(list_policies))
        .route("/api/forensics/observability/panels", get(panels))
        .route("/api/forensics/observability/alerts", get(alerts))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-forensics",
        "status": "ok",
        "upstream": "cilium/tetragon v1.7.0"
    }))
}

async fn list_cases(extract::State(state): extract::State<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "cases": state.cases.list(),
        "count": state.cases.count(),
    }))
}

async fn list_policies(extract::State(state): extract::State<Arc<State>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "policies": state.policies.list(),
        "count": state.policies.count(),
    }))
}

async fn panels() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "panels": crate::observability::dashboard_panels(),
    }))
}

async fn alerts() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "alerts": crate::observability::alert_rules(),
    }))
}
