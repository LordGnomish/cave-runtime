//! HTTP routes for cave-portal.

use crate::State;
use axum::{
    extract::State as AxumState,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/portal/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-portal",
        "status": "ok",
        "upstream": "Backstage"
    }))
}
