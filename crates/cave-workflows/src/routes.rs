// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP routes for cave-workflows.

use crate::State;
use axum::{
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/workflows/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-workflows",
        "status": "ok",
        "upstream": "n8n"
    }))
}
