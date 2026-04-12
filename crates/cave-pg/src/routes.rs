use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::backup::BackupManager;
use crate::ha::HaController;
use crate::lifecycle::InstanceManager;
use crate::monitoring::Monitor;
use crate::pool::ConnectionPool;
use crate::types::{BackupType, PoolConfig, PoolMode, PgRole};
use crate::user::UserManager;

// ── State ────────────────────────────────────────────────────────────────────

pub struct PgState {
    pub instances: Arc<InstanceManager>,
    pub pools: Arc<ConnectionPool>,
    pub backups: Arc<BackupManager>,
    pub ha: Arc<HaController>,
    pub users: Arc<UserManager>,
}

// ── Request / response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateInstanceRequest {
    pub name: String,
    pub version: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
}

#[derive(Deserialize)]
pub struct CreatePoolRequest {
    pub name: String,
    pub instance_id: String,
    pub mode: Option<String>,
    pub pool_size: Option<u32>,
    pub min_pool_size: Option<u32>,
    pub max_client_connections: Option<u32>,
}

#[derive(Deserialize)]
pub struct StartBackupRequest {
    pub instance_id: String,
    #[serde(rename = "type")]
    pub backup_type: Option<String>,
}

#[derive(Deserialize)]
pub struct FailoverRequest {
    pub primary_id: String,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
pub struct InstanceIdQuery {
    pub instance_id: Option<String>,
}

#[derive(Deserialize)]
pub struct PrimaryIdQuery {
    pub primary_id: Option<String>,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub module: &'static str,
}

// ── Router ───────────────────────────────────────────────────────────────────

pub fn router(state: Arc<PgState>) -> Router {
    Router::new()
        .route("/api/pg/instances", get(list_instances).post(create_instance))
        .route(
            "/api/pg/instances/:id",
            get(get_instance).delete(delete_instance),
        )
        .route("/api/pg/instances/:id/start", post(start_instance))
        .route("/api/pg/instances/:id/stop", post(stop_instance))
        .route("/api/pg/instances/:id/restart", post(restart_instance))
        .route("/api/pg/pools", get(list_pools).post(create_pool))
        .route("/api/pg/pools/:name", delete(remove_pool))
        .route("/api/pg/pools/:name/stats", get(pool_stats))
        .route("/api/pg/backups", get(list_backups).post(start_backup))
        .route("/api/pg/ha/replicas", get(list_replicas))
        .route("/api/pg/ha/failover", post(trigger_failover))
        .route("/api/pg/monitoring/:id/activity", get(get_activity))
        .route("/api/pg/users/roles", get(list_roles).post(create_role))
        .route("/api/pg/health", get(health))
        .with_state(state)
}

// ── Instance handlers ─────────────────────────────────────────────────────────

async fn list_instances(State(state): State<Arc<PgState>>) -> impl IntoResponse {
    let instances = state.instances.list_instances();
    Json(serde_json::json!({ "instances": instances }))
}

async fn create_instance(
    State(state): State<Arc<PgState>>,
    Json(req): Json<CreateInstanceRequest>,
) -> impl IntoResponse {
    match state.instances.create_instance(
        &req.name,
        &req.version,
        req.host.as_deref().unwrap_or("localhost"),
        req.port.unwrap_or(5432),
        req.database.as_deref().unwrap_or("postgres"),
        req.username.as_deref().unwrap_or("postgres"),
    ) {
        Ok(instance) => (StatusCode::CREATED, Json(serde_json::json!(instance))).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn get_instance(
    State(state): State<Arc<PgState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.instances.get_instance(&id) {
        Ok(i) => Json(serde_json::json!(i)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn delete_instance(
    State(state): State<Arc<PgState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.instances.delete_instance(&id) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn start_instance(
    State(state): State<Arc<PgState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.instances.start_instance(&id) {
        Ok(_) => Json(serde_json::json!({"status": "running"})).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn stop_instance(
    State(state): State<Arc<PgState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.instances.stop_instance(&id) {
        Ok(_) => Json(serde_json::json!({"status": "stopped"})).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn restart_instance(
    State(state): State<Arc<PgState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.instances.restart_instance(&id) {
        Ok(_) => Json(serde_json::json!({"status": "running"})).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Pool handlers ─────────────────────────────────────────────────────────────

async fn list_pools(State(state): State<Arc<PgState>>) -> impl IntoResponse {
    let pools = state.pools.list_pools();
    Json(serde_json::json!({ "pools": pools }))
}

async fn create_pool(
    State(state): State<Arc<PgState>>,
    Json(req): Json<CreatePoolRequest>,
) -> impl IntoResponse {
    let mode = match req.mode.as_deref().unwrap_or("transaction") {
        "session" => PoolMode::Session,
        "statement" => PoolMode::Statement,
        _ => PoolMode::Transaction,
    };
    let config = PoolConfig {
        name: req.name.clone(),
        instance_id: req.instance_id,
        mode,
        pool_size: req.pool_size.unwrap_or(20),
        min_pool_size: req.min_pool_size.unwrap_or(5),
        max_client_connections: req.max_client_connections.unwrap_or(100),
        server_idle_timeout_secs: 600,
        client_idle_timeout_secs: 0,
    };
    match state.pools.create_pool(config) {
        Ok(_) => (StatusCode::CREATED, Json(serde_json::json!({"name": req.name}))).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn remove_pool(
    State(state): State<Arc<PgState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.pools.remove_pool(&name) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

async fn pool_stats(
    State(state): State<Arc<PgState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.pools.get_stats(&name) {
        Ok(stats) => Json(serde_json::json!(stats)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Backup handlers ───────────────────────────────────────────────────────────

async fn list_backups(
    State(state): State<Arc<PgState>>,
    Query(params): Query<InstanceIdQuery>,
) -> impl IntoResponse {
    let instance_id = params.instance_id.as_deref().unwrap_or("");
    let backups = state.backups.list_backups(instance_id);
    Json(serde_json::json!({ "backups": backups }))
}

async fn start_backup(
    State(state): State<Arc<PgState>>,
    Json(req): Json<StartBackupRequest>,
) -> impl IntoResponse {
    let btype = match req.backup_type.as_deref().unwrap_or("Full") {
        "Incremental" => BackupType::Incremental,
        "WAL" => BackupType::WAL,
        _ => BackupType::Full,
    };
    match state.backups.start_backup(&req.instance_id, btype) {
        Ok(b) => (StatusCode::CREATED, Json(serde_json::json!(b))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── HA handlers ───────────────────────────────────────────────────────────────

async fn list_replicas(
    State(state): State<Arc<PgState>>,
    Query(params): Query<PrimaryIdQuery>,
) -> impl IntoResponse {
    let primary_id = params.primary_id.as_deref().unwrap_or("");
    let replicas = state.ha.list_replicas(primary_id);
    Json(serde_json::json!({ "replicas": replicas }))
}

async fn trigger_failover(
    State(state): State<Arc<PgState>>,
    Json(req): Json<FailoverRequest>,
) -> impl IntoResponse {
    let reason = req.reason.as_deref().unwrap_or("manual");
    match state.ha.trigger_failover(&req.primary_id, reason) {
        Ok(event) => Json(serde_json::json!(event)).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Monitoring handlers ───────────────────────────────────────────────────────

async fn get_activity(
    State(_state): State<Arc<PgState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Return mock data in tests / demo
    let mock_activities = vec![
        serde_json::json!({
            "pid": 1234,
            "datname": "postgres",
            "usename": "app",
            "state": "active",
            "query": "SELECT * FROM users",
            "duration_ms": 5
        }),
        serde_json::json!({
            "pid": 1235,
            "datname": "postgres",
            "usename": "app",
            "state": "idle",
            "query": "",
            "duration_ms": 0
        }),
    ];
    let rows: Vec<std::collections::HashMap<String, serde_json::Value>> = mock_activities
        .iter()
        .map(|v| {
            v.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .collect();
    let activities = Monitor::parse_stat_activity(&rows);
    let metrics = Monitor::metrics_text(&id, &activities, &[]);
    Json(serde_json::json!({ "activities": activities, "metrics": metrics }))
}

// ── User handlers ─────────────────────────────────────────────────────────────

async fn list_roles(State(state): State<Arc<PgState>>) -> impl IntoResponse {
    let roles = state.users.list_roles();
    Json(serde_json::json!({ "roles": roles }))
}

async fn create_role(
    State(state): State<Arc<PgState>>,
    Json(role): Json<PgRole>,
) -> impl IntoResponse {
    match state.users.create_role(role) {
        Ok(sql) => (StatusCode::CREATED, Json(serde_json::json!({"sql": sql}))).into_response(),
        Err(e) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Health ────────────────────────────────────────────────────────────────────

async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        module: crate::MODULE_NAME,
    })
}
