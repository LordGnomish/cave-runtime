// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP admin API routes for cave-docdb.

use crate::bson::Document;
use crate::cursor::CursorStore;
use crate::engine::Engine;
use crate::models::*;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

#[derive(Clone)]
pub struct DocDbState {
    pub engine: Arc<Engine>,
    pub cursors: Arc<CursorStore>,
    pub wire_port: Arc<AtomicU16>,
}

impl Default for DocDbState {
    fn default() -> Self {
        Self {
            engine: Arc::new(Engine::new()),
            cursors: Arc::new(CursorStore::new()),
            wire_port: Arc::new(AtomicU16::new(27017)),
        }
    }
}

pub fn create_router(state: Arc<DocDbState>) -> Router {
    Router::new()
        .route("/api/docdb/health", get(health))
        .route("/api/docdb/databases", get(list_databases))
        .route(
            "/api/docdb/databases/{db}/collections",
            get(list_collections),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/stats",
            get(collection_stats),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/find",
            post(find),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/insert",
            post(insert),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/update",
            post(update),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/delete",
            post(delete),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/aggregate",
            post(aggregate),
        )
        .route(
            "/api/docdb/databases/{db}/collections/{col}/indexes",
            get(list_indexes).post(create_indexes),
        )
        .route("/api/docdb/stats", get(engine_stats))
        .route("/api/docdb/server/info", get(server_info))
        .route("/api/docdb/server/port", get(server_port))
        .with_state(state)
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<Value>)>;

fn err_not_found(msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn err_bad_request(msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn err_internal(msg: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg })),
    )
}

async fn health(State(_state): State<Arc<DocDbState>>) -> ApiResult<HealthResponse> {
    Ok(Json(HealthResponse {
        status: "ok".to_string(),
    }))
}

async fn list_databases(State(state): State<Arc<DocDbState>>) -> ApiResult<Vec<String>> {
    state
        .engine
        .list_databases()
        .await
        .map(Json)
        .map_err(|e| err_internal(&e))
}

async fn list_collections(
    State(state): State<Arc<DocDbState>>,
    Path(db): Path<String>,
) -> ApiResult<Vec<String>> {
    match state.engine.get_database(&db).await {
        Some(database) => database
            .list_collections()
            .await
            .map(Json)
            .map_err(|e| err_internal(&e)),
        None => Err(err_not_found("database not found")),
    }
}

async fn collection_stats(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
) -> ApiResult<CollectionStats> {
    match state.engine.get_database(&db).await {
        Some(database) => match database.get_collection(&col).await {
            Some(collection) => {
                let stats = collection.stats().await.map_err(|e| err_internal(&e))?;
                Ok(Json(CollectionStats {
                    name: col,
                    document_count: stats.document_count,
                    index_count: stats.index_count,
                }))
            }
            None => Err(err_not_found("collection not found")),
        },
        None => Err(err_not_found("database not found")),
    }
}

async fn find(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(req): Json<FindRequest>,
) -> ApiResult<FindResponse> {
    let filter = req.filter.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let docs = collection
        .find(filter.as_ref())
        .await
        .map_err(|e| err_internal(&e))?;

    let count = docs.len();
    let documents = docs
        .into_iter()
        .map(|doc| Value::Object(doc.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
        .collect();

    Ok(Json(FindResponse { documents, count }))
}

async fn insert(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(req): Json<InsertRequest>,
) -> ApiResult<InsertResponse> {
    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let mut docs = Vec::new();
    for doc_val in req.documents {
        if let Value::Object(obj) = doc_val {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k, v);
            }
            docs.push(doc);
        }
    }

    let inserted_ids = collection
        .insert_many(docs)
        .await
        .map_err(|e| err_internal(&e))?;

    let inserted_count = inserted_ids.len();

    Ok(Json(InsertResponse {
        inserted_ids,
        inserted_count,
    }))
}

async fn update(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(req): Json<UpdateRequest>,
) -> ApiResult<UpdateResponse> {
    let filter = req.filter.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let mut update = Document::new();
    if let Value::Object(obj) = &req.update {
        for (k, v) in obj {
            update.insert(k.clone(), v.clone());
        }
    } else {
        return Err(err_bad_request("invalid update spec"));
    }

    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let modified_count = collection
        .update_many(filter.as_ref(), &update)
        .await
        .map_err(|e| err_internal(&e))?;

    Ok(Json(UpdateResponse { modified_count }))
}

async fn delete(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(req): Json<DeleteRequest>,
) -> ApiResult<DeleteResponse> {
    let filter = req.filter.as_ref().and_then(|f| {
        if let Value::Object(obj) = f {
            let mut doc = Document::new();
            for (k, v) in obj {
                doc.insert(k.clone(), v.clone());
            }
            Some(doc)
        } else {
            None
        }
    });

    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let deleted_count = collection
        .delete_many(filter.as_ref())
        .await
        .map_err(|e| err_internal(&e))?;

    Ok(Json(DeleteResponse { deleted_count }))
}

async fn aggregate(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(_req): Json<AggregateRequest>,
) -> ApiResult<Vec<Value>> {
    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let docs = collection.find(None).await.map_err(|e| err_internal(&e))?;

    let results = docs
        .into_iter()
        .map(|doc| Value::Object(doc.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
        .collect();

    Ok(Json(results))
}

async fn list_indexes(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
) -> ApiResult<Vec<String>> {
    match state.engine.get_database(&db).await {
        Some(database) => match database.get_collection(&col).await {
            Some(collection) => {
                let indexes = collection
                    .list_indexes()
                    .await
                    .map_err(|e| err_internal(&e))?;
                let names: Vec<String> = indexes.iter().map(|idx| idx.name.clone()).collect();
                Ok(Json(names))
            }
            None => Err(err_not_found("collection not found")),
        },
        None => Err(err_not_found("database not found")),
    }
}

async fn create_indexes(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(req): Json<IndexCreateRequest>,
) -> ApiResult<Value> {
    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let mut keys = std::collections::BTreeMap::new();
    for (k, v) in &req.keys {
        if let Some(n) = v.as_i64() {
            keys.insert(k.clone(), n as i32);
        }
    }

    let name = req.name.unwrap_or_else(|| "index".to_string());
    let unique = req.unique.unwrap_or(false);

    let index = crate::index::Index::new(name.clone(), keys, unique);
    collection
        .add_index(index)
        .await
        .map_err(|e| err_internal(&e))?;

    Ok(Json(serde_json::json!({ "index_name": name })))
}

async fn engine_stats(State(state): State<Arc<DocDbState>>) -> ApiResult<Value> {
    let stats = state.engine.stats().await.map_err(|e| err_internal(&e))?;
    Ok(Json(serde_json::json!({
        "database_count": stats.database_count,
        "collection_count": stats.collection_count,
        "document_count": stats.document_count,
    })))
}

async fn server_info() -> ApiResult<Value> {
    Ok(Json(serde_json::json!({
        "version": "6.0.0",
        "pid": std::process::id(),
        "uptime_seconds": 0,
    })))
}

async fn server_port(State(state): State<Arc<DocDbState>>) -> ApiResult<Value> {
    let port = state.wire_port.load(Ordering::SeqCst);
    Ok(Json(serde_json::json!({ "port": port })))
}
