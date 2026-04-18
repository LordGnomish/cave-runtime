//! AlertManager-compatible HTTP API routes.
//! Implements: /api/v2/alerts, /api/v2/silences, /api/v2/status, /api/v2/receivers

use axum::{
    extract::{Path, State},
    routing::{delete, get, post},
    Json, Router,
};
use std::sync::Arc;
use crate::alertmgr::{model::{Alert, Silence, SilenceStatus}, silence::SilenceStore};

pub struct AlertmgrState {
    pub silences: Arc<SilenceStore>,
    pub alerts: Arc<parking_lot::RwLock<Vec<Alert>>>,
}

pub fn create_router(state: Arc<AlertmgrState>) -> Router {
    Router::new()
        .route("/api/v2/alerts",            get(get_alerts).post(post_alerts))
        .route("/api/v2/silences",          get(get_silences).post(create_silence))
        .route("/api/v2/silence/{id}",       delete(delete_silence))
        .route("/api/v2/status",            get(get_status))
        .route("/api/v2/receivers",         get(get_receivers))
        .with_state(state)
}

async fn get_alerts(State(s): State<Arc<AlertmgrState>>) -> Json<Vec<Alert>> {
    Json(s.alerts.read().clone())
}

async fn post_alerts(
    State(s): State<Arc<AlertmgrState>>,
    Json(alerts): Json<Vec<Alert>>,
) -> Json<serde_json::Value> {
    s.alerts.write().extend(alerts);
    Json(serde_json::json!({}))
}

async fn get_silences(State(s): State<Arc<AlertmgrState>>) -> Json<Vec<Silence>> {
    Json(s.silences.list())
}

async fn create_silence(
    State(s): State<Arc<AlertmgrState>>,
    Json(mut silence): Json<Silence>,
) -> Json<serde_json::Value> {
    let id = s.silences.create(silence);
    Json(serde_json::json!({ "silenceID": id }))
}

async fn delete_silence(
    State(s): State<Arc<AlertmgrState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    s.silences.expire(&id);
    Json(serde_json::json!({}))
}

async fn get_status() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "cluster": { "name": "cave-metrics", "status": "ready", "peers": [] },
        "versionInfo": { "branch": "main", "version": "0.1.0" },
        "config": { "original": "" },
        "uptime": chrono::Utc::now().to_rfc3339()
    }))
}

async fn get_receivers() -> Json<serde_json::Value> {
    Json(serde_json::json!([{ "name": "default", "webhookConfigs": [] }]))
}
