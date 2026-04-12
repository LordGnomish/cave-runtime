//! HTTP routes for cave-ai-obs.

use crate::State;
use axum::{
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/ai-obs/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-ai-obs",
        "status": "ok",
        "upstream": "Langfuse"
    }))
}
