//! HTTP routes for cave-ebpf-common.

use crate::State;
use axum::{
    extract::State as AxumState,
    routing::get,
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        .route("/api/ebpf-common/health", get(health))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-ebpf-common",
        "status": "ok",
        "upstream": "eBPF"
    }))
}
