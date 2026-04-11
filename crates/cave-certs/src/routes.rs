//! HTTP routes for cave-certs.

use crate::State;
use axum::{
    extract::State as AxumState,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/certs/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-certs",
        "status": "ok",
        "upstream": "cert-manager"
    }))
}
