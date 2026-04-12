//! HTTP routes for cave-registry.

use crate::State;
use axum::{
    routing::get,
    Json, Router,
};
use axum::{routing::get, Json, Router};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/registry/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-registry",
        "status": "ok",
        "upstream": "Pulp"
    }))
}
