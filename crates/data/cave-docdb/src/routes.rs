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
            "/api/docdb/databases/{db}/collections/{col}/text",
            post(text_search),
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

    let mut docs = collection
        .find(filter.as_ref())
        .await
        .map_err(|e| err_internal(&e))?;

    // sort -> skip -> limit -> projection (MongoDB query-stage order).
    if let Some(Value::Object(sort)) = req.sort.as_ref() {
        sort_documents(&mut docs, sort);
    }
    if let Some(skip) = req.skip {
        if skip > 0 {
            docs = docs.into_iter().skip(skip as usize).collect();
        }
    }
    if let Some(limit) = req.limit {
        if limit >= 0 {
            docs.truncate(limit as usize);
        }
    }

    let projection = req.projection.as_ref().and_then(|p| {
        p.as_object()
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Document>())
    });

    let count = docs.len();
    let documents = docs
        .into_iter()
        .map(|doc| {
            let projected = crate::projection::apply_projection(&doc, projection.as_ref());
            Value::Object(projected.into_iter().collect())
        })
        .collect();

    Ok(Json(FindResponse { documents, count }))
}

/// Stable multi-key sort honoring a `{field: 1|-1}` sort spec.
fn sort_documents(docs: &mut [Document], sort: &serde_json::Map<String, Value>) {
    docs.sort_by(|a, b| {
        for (key, dir) in sort {
            let d = dir.as_i64().unwrap_or(1);
            let av = a.get(key);
            let bv = b.get(key);
            let ord = cmp_opt(av, bv);
            if ord != std::cmp::Ordering::Equal {
                return if d >= 0 { ord } else { ord.reverse() };
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn cmp_opt(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => {
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                xf.partial_cmp(&yf).unwrap_or(Ordering::Equal)
            } else if let (Some(xs), Some(ys)) = (x.as_str(), y.as_str()) {
                xs.cmp(ys)
            } else {
                Ordering::Equal
            }
        }
    }
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
    Json(req): Json<AggregateRequest>,
) -> ApiResult<Vec<Value>> {
    // Build the command document the aggregation engine expects and run the
    // full pipeline ($match/$project/$group/$sort/$unwind/$limit/$skip/...).
    let mut cmd = Document::new();
    cmd.insert("aggregate".to_string(), Value::String(col.clone()));
    cmd.insert("$db".to_string(), Value::String(db.clone()));
    cmd.insert("pipeline".to_string(), Value::Array(req.pipeline));

    let resp = crate::commands::agg::aggregate(&cmd, state.engine.clone())
        .await
        .map_err(|e| err_internal(&e))?;

    let results = resp
        .get("cursor")
        .and_then(|v| v.as_object())
        .and_then(|c| c.get("firstBatch"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(Json(results))
}

async fn text_search(
    State(state): State<Arc<DocDbState>>,
    Path((db, col)): Path<(String, String)>,
    Json(req): Json<TextSearchRequest>,
) -> ApiResult<FindResponse> {
    let filter = req.filter.as_ref().and_then(|f| {
        f.as_object().map(|obj| {
            obj.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Document>()
        })
    });

    let database = state.engine.get_or_create_database(&db).await;
    let collection = database.get_or_create_collection(&col).await;

    let docs = collection
        .text_search(&req.search, filter.as_ref())
        .await
        .map_err(|e| err_bad_request(&e))?;

    let count = docs.len();
    let documents = docs
        .into_iter()
        .map(|doc| Value::Object(doc.iter().map(|(k, v)| (k.clone(), v.clone())).collect()))
        .collect();

    Ok(Json(FindResponse { documents, count }))
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

    // A `{field: "text"}` key declares a text index; numeric keys are b-tree.
    let text_fields: Vec<String> = req
        .keys
        .iter()
        .filter(|(_, v)| v.as_str() == Some("text"))
        .map(|(k, _)| k.clone())
        .collect();

    let mut keys = std::collections::BTreeMap::new();
    for (k, v) in &req.keys {
        if let Some(n) = v.as_i64() {
            keys.insert(k.clone(), n as i32);
        }
    }

    let unique = req.unique.unwrap_or(false);

    let index = if !text_fields.is_empty() {
        let name = req.name.unwrap_or_else(|| "text_index".to_string());
        (name.clone(), crate::index::Index::text(name, text_fields))
    } else {
        let name = req.name.unwrap_or_else(|| "index".to_string());
        (name.clone(), crate::index::Index::new(name, keys, unique))
    };
    let (name, index) = index;
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(state: &Arc<DocDbState>, db: &str, col: &str, docs: Vec<Document>) {
        let database = state.engine.get_or_create_database(db).await;
        let collection = database.get_or_create_collection(col).await;
        collection.insert_many(docs).await.unwrap();
    }

    fn doc(json: Value) -> Document {
        json.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    #[tokio::test]
    async fn aggregate_route_applies_unwind_pipeline() {
        let state = Arc::new(DocDbState::default());
        seed(
            &state,
            "rdb",
            "items",
            vec![doc(serde_json::json!({"_id": 1, "tags": ["a", "b", "c"]}))],
        )
        .await;

        let Json(results) = aggregate(
            State(state.clone()),
            Path(("rdb".to_string(), "items".to_string())),
            Json(AggregateRequest {
                pipeline: vec![serde_json::json!({"$unwind": "$tags"})],
            }),
        )
        .await
        .unwrap();

        // The route must run the pipeline: one array of 3 tags -> 3 documents.
        assert_eq!(results.len(), 3);
        let tags: Vec<&str> = results
            .iter()
            .filter_map(|d| d.get("tags").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(tags, vec!["a", "b", "c"]);
    }

    #[tokio::test]
    async fn find_route_applies_projection_sort_skip_limit() {
        let state = Arc::new(DocDbState::default());
        seed(
            &state,
            "fdb",
            "users",
            vec![
                doc(serde_json::json!({"name": "c", "age": 30, "secret": "x"})),
                doc(serde_json::json!({"name": "a", "age": 10, "secret": "y"})),
                doc(serde_json::json!({"name": "b", "age": 20, "secret": "z"})),
            ],
        )
        .await;

        let Json(resp) = find(
            State(state.clone()),
            Path(("fdb".to_string(), "users".to_string())),
            Json(FindRequest {
                filter: Some(serde_json::json!({})),
                projection: Some(serde_json::json!({"name": 1, "age": 1, "_id": 0})),
                limit: Some(2),
                skip: Some(1),
                sort: Some(serde_json::json!({"age": 1})),
            }),
        )
        .await
        .unwrap();

        // sorted by age asc -> a(10), b(20), c(30); skip 1 -> b,c; limit 2 -> b,c.
        assert_eq!(resp.count, 2);
        let names: Vec<&str> = resp
            .documents
            .iter()
            .filter_map(|d| d.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(names, vec!["b", "c"]);
        // projection drops `secret` and `_id`.
        assert!(resp.documents[0].get("secret").is_none());
        assert!(resp.documents[0].get("_id").is_none());
        assert!(resp.documents[0].get("age").is_some());
    }

    #[tokio::test]
    async fn text_search_route_uses_text_index() {
        let state = Arc::new(DocDbState::default());
        seed(
            &state,
            "tdb",
            "docs",
            vec![
                doc(serde_json::json!({"title": "Cold Brew", "body": "iced coffee"})),
                doc(serde_json::json!({"title": "Hot Tea", "body": "warm tea"})),
            ],
        )
        .await;

        // Create the text index through the index route.
        create_indexes(
            State(state.clone()),
            Path(("tdb".to_string(), "docs".to_string())),
            Json(IndexCreateRequest {
                keys: serde_json::json!({"title": "text", "body": "text"})
                    .as_object()
                    .unwrap()
                    .clone(),
                unique: None,
                name: None,
            }),
        )
        .await
        .unwrap();

        let Json(resp) = text_search(
            State(state.clone()),
            Path(("tdb".to_string(), "docs".to_string())),
            Json(TextSearchRequest {
                search: "coffee".to_string(),
                filter: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(resp.count, 1);
        assert_eq!(
            resp.documents[0].get("title").and_then(|v| v.as_str()),
            Some("Cold Brew")
        );
    }

    #[tokio::test]
    async fn aggregate_route_applies_match_pipeline() {
        let state = Arc::new(DocDbState::default());
        seed(
            &state,
            "rdb2",
            "scores",
            vec![
                doc(serde_json::json!({"v": 10})),
                doc(serde_json::json!({"v": 50})),
                doc(serde_json::json!({"v": 90})),
            ],
        )
        .await;

        let Json(results) = aggregate(
            State(state.clone()),
            Path(("rdb2".to_string(), "scores".to_string())),
            Json(AggregateRequest {
                pipeline: vec![serde_json::json!({"$match": {"v": {"$gte": 50}}})],
            }),
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 2);
    }
}
