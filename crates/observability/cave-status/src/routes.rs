// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-status.

use axum::{Json, Router, routing::get};

pub fn create_router() -> Router {
    Router::new().route("/api/status/health", get(health))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-status",
        "status": "ok",
        "upstream": "custom",
        "features": "Public/internal status page, auto-generation from probes, incident integration"
    }))
}
