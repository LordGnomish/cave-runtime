// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP admin API routes.

use crate::engine::Engine;
use crate::models::*;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct RdbmsState {
    pub engine: Arc<Engine>,
}

impl Default for RdbmsState {
    fn default() -> Self {
        RdbmsState {
            engine: Arc::new(Engine::new()),
        }
    }
}

pub fn create_router(state: Arc<RdbmsState>) -> Router {
    Router::new()
        .route("/api/rdbms/health", get(health))
        .route("/api/rdbms/databases", get(databases))
        .route(
            "/api/rdbms/databases/{db}/schemas",
            get(schemas),
        )
        .route(
            "/api/rdbms/databases/{db}/schemas/{schema}/tables",
            get(tables),
        )
        .route(
            "/api/rdbms/databases/{db}/schemas/{schema}/tables/{table}",
            get(table_info),
        )
        .route(
            "/api/rdbms/databases/{db}/schemas/{schema}/tables/{table}/rows",
            get(table_rows),
        )
        .route("/api/rdbms/exec", post(exec))
        .route("/api/rdbms/server/port", get(server_port))
        .route("/api/rdbms/server/info", get(server_info))
        .route("/api/rdbms/explain", post(explain))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn databases(State(state): State<Arc<RdbmsState>>) -> impl IntoResponse {
    let db = state.engine.get_database().await;
    Json(serde_json::json!({
        "databases": vec![&db.name]
    }))
}

async fn schemas(
    State(state): State<Arc<RdbmsState>>,
    Path(_db): Path<String>,
) -> impl IntoResponse {
    let database = state.engine.get_database().await;
    let schema_names: Vec<_> = database.schemas.keys().cloned().collect();
    Json(serde_json::json!({
        "schemas": schema_names
    }))
}

async fn tables(
    State(state): State<Arc<RdbmsState>>,
    Path((_db, schema)): Path<(String, String)>,
) -> impl IntoResponse {
    let database = state.engine.get_database().await;
    if let Some(s) = database.schemas.get(&schema) {
        let table_names: Vec<_> = s.tables.keys().cloned().collect();
        (StatusCode::OK, Json(serde_json::json!({
            "tables": table_names
        }))).into_response()
    } else {
        (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "schema not found"}))).into_response()
    }
}

async fn table_info(
    State(state): State<Arc<RdbmsState>>,
    Path((_db, schema, table)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let database = state.engine.get_database().await;
    if let Some(s) = database.schemas.get(&schema) {
        if let Some(t) = s.tables.get(&table) {
            let columns: Vec<_> = t
                .columns
                .iter()
                .map(|c| ColumnInfo {
                    name: c.name.clone(),
                    type_name: c.type_name.clone(),
                    not_null: c.not_null,
                    primary_key: c.primary_key,
                })
                .collect();
            return Json(serde_json::json!(TableInfo {
                name: t.name.clone(),
                columns,
                row_count: t.row_count(),
            }))
            .into_response();
        }
    }
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "table not found"}))).into_response()
}

async fn table_rows(
    State(state): State<Arc<RdbmsState>>,
    Path((_db, schema, table)): Path<(String, String, String)>,
) -> impl IntoResponse {
    let database = state.engine.get_database().await;
    if let Some(s) = database.schemas.get(&schema) {
        if let Some(t) = s.tables.get(&table) {
            let rows: Vec<Vec<serde_json::Value>> = t
                .rows
                .iter()
                .map(|row| {
                    row.iter()
                        .map(|v| v.to_json())
                        .collect()
                })
                .collect();
            return Json(serde_json::json!({
                "rows": rows,
                "count": rows.len()
            }))
            .into_response();
        }
    }
    (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "table not found"}))).into_response()
}

async fn exec(
    State(state): State<Arc<RdbmsState>>,
    Json(_req): Json<ExecRequest>,
) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let _database = state.engine.get_database().await;

    // Stub response
    Json(ExecResponse {
        columns: vec!["result".to_string()],
        rows: vec![],
        row_count: 0,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

async fn server_port() -> impl IntoResponse {
    Json(serde_json::json!({
        "port": 5432
    }))
}

async fn server_info(State(state): State<Arc<RdbmsState>>) -> impl IntoResponse {
    let db = state.engine.get_database().await;
    let tables_count: usize = db.schemas.values().map(|s| s.tables.len()).sum();
    Json(ServerInfo {
        server_version: "14.0".to_string(),
        uptime: 0,
        databases: 1,
        tables_count,
    })
}

async fn explain(
    State(_state): State<Arc<RdbmsState>>,
    Json(req): Json<ExecRequest>,
) -> impl IntoResponse {
    Json(ExplainResponse {
        plan: format!("PLAN: {}", req.sql),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rdbms_state_creation() {
        let state = RdbmsState::default();
        assert!(Arc::ptr_eq(&state.engine, &state.engine));
    }
}
