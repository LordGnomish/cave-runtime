//! HTTP route handlers for cave-search.
//!
//! Provides an OpenSearch/Elasticsearch-compatible REST API:
//!
//! | Method | Path                                        | Description             |
//! |--------|---------------------------------------------|-------------------------|
//! | PUT    | /api/search/{index}                         | Create index            |
//! | DELETE | /api/search/{index}                         | Delete index            |
//! | GET    | /api/search/{index}                         | Get index info          |
//! | GET    | /api/search/{index}/_stats                  | Index statistics        |
//! | GET    | /api/search/{index}/_count                  | Document count          |
//! | POST   | /api/search/{index}/_doc                    | Index document (auto-id)|
//! | PUT    | /api/search/{index}/_doc/{id}               | Index document (with id)|
//! | GET    | /api/search/{index}/_doc/{id}               | Get document            |
//! | DELETE | /api/search/{index}/_doc/{id}               | Delete document         |
//! | POST   | /api/search/{index}/_search                 | Search                  |
//! | GET    | /api/search/{index}/_search                 | Search (GET)            |
//! | POST   | /api/search/_bulk                           | Bulk operations         |
//! | GET    | /api/search/_cat/indices                    | List indices (cat API)  |
//! | GET    | /api/search/_aliases                        | List aliases            |
//! | GET    | /api/search/health                          | Health check            |

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::models::{
    BulkActionMeta, CatIndexRow, CountResponse, CreateIndexRequest, Document, SearchRequest,
    ShardStats,
};
use crate::{SearchError, SearchState};

// ─────────────────────────────────────────────────────────────────────────────
// Router factory
// ─────────────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<SearchState>) -> Router {
    Router::new()
        // Health
        .route("/api/search/health", get(health))
        // Cat API
        .route("/api/search/_cat/indices", get(cat_indices))
        // Aliases
        .route("/api/search/_aliases", get(list_aliases))
        // Bulk
        .route("/api/search/_bulk", post(bulk))
        .route("/api/search/{index}/_bulk", post(bulk_index))
        // Index lifecycle
        .route("/api/search/{index}", put(create_index))
        .route("/api/search/{index}", delete(delete_index))
        .route("/api/search/{index}", get(get_index))
        // Stats and count
        .route("/api/search/{index}/_stats", get(index_stats))
        .route("/api/search/{index}/_count", get(count))
        // Document CRUD
        .route("/api/search/{index}/_doc", post(index_doc_auto))
        .route("/api/search/{index}/_doc/{id}", put(index_doc_with_id))
        .route("/api/search/{index}/_doc/{id}", get(get_doc))
        .route("/api/search/{index}/_doc/{id}", delete(delete_doc))
        // Search
        .route("/api/search/{index}/_search", post(search))
        .route("/api/search/{index}/_search", get(search_get))
        // Refresh
        .route("/api/search/{index}/_refresh", post(refresh_index))
        // Mapping
        .route("/api/search/{index}/_mapping", get(get_mapping))
        .route("/api/search/{index}/_mapping", put(put_mapping))
        .with_state(state)
}

// ─────────────────────────────────────────────────────────────────────────────
// Health
// ─────────────────────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<SearchState>>) -> Json<Value> {
    let index_count = state.engine.list_indices().len();
    Json(json!({
        "module": "cave-search",
        "status": "green",
        "upstream": "OpenSearch / Elasticsearch",
        "implementation": "built-in inverted index (BM25)",
        "indices": index_count,
        "features": [
            "inverted-index",
            "bm25-scoring",
            "bool-query",
            "match-query",
            "multi-match",
            "term-query",
            "terms-query",
            "range-query",
            "prefix-query",
            "fuzzy-query",
            "wildcard-query",
            "exists-query",
            "ids-query",
            "query-string",
            "aggregations",
            "highlighting",
            "sorting",
            "pagination",
            "bulk-api",
            "tenant-isolation"
        ]
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Index lifecycle
// ─────────────────────────────────────────────────────────────────────────────

async fn create_index(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
    body: Option<Json<CreateIndexRequest>>,
) -> impl IntoResponse {
    let req = body.map(|b| b.0).unwrap_or_default();

    match state.engine.create_index(
        &index,
        req.mappings,
        req.settings,
        req.aliases,
    ) {
        Ok(()) => {
            info!(index = %index, "index created");
            (StatusCode::OK, Json(json!({
                "acknowledged": true,
                "shards_acknowledged": true,
                "index": index
            })))
        }
        Err(SearchError::IndexAlreadyExists(_)) => {
            (StatusCode::BAD_REQUEST, Json(json!({
                "error": {
                    "type": "resource_already_exists_exception",
                    "reason": format!("index [{}] already exists", index),
                    "index": index
                },
                "status": 400
            })))
        }
        Err(e) => {
            warn!(index = %index, error = %e, "failed to create index");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
                "error": {"type": "internal_error", "reason": e.to_string()},
                "status": 500
            })))
        }
    }
}

async fn delete_index(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.delete_index(&index) {
        Ok(()) => (StatusCode::OK, Json(json!({"acknowledged": true}))),
        Err(SearchError::IndexNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"type": "index_not_found_exception", "reason": format!("no such index [{}]", index)}, "status": 404})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"type": "internal_error", "reason": e.to_string()}, "status": 500})),
        ),
    }
}

async fn get_index(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.get_index_info(&index) {
        Ok(info) => {
            let mut resp = HashMap::new();
            resp.insert(index.clone(), info);
            (StatusCode::OK, Json(serde_json::to_value(resp).unwrap_or_default()))
        }
        Err(SearchError::IndexNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"type": "index_not_found_exception", "reason": format!("no such index [{}]", index)}, "status": 404})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"type": "internal_error", "reason": e.to_string()}, "status": 500})),
        ),
    }
}

async fn index_stats(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.get_index_stats(&index) {
        Ok(stats) => (StatusCode::OK, Json(json!({
            "_all": {
                "primaries": {
                    "docs": {"count": stats.doc_count, "deleted": stats.deleted_count},
                    "store": {"size_in_bytes": stats.store_size_bytes},
                    "indexing": {"index_total": stats.index_total, "index_time_in_millis": stats.index_time_ms},
                    "search": {"query_total": stats.search_total, "query_time_in_millis": stats.search_time_ms}
                }
            },
            "indices": {
                index: {
                    "primaries": {
                        "docs": {"count": stats.doc_count, "deleted": stats.deleted_count}
                    }
                }
            }
        }))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"type": "index_not_found_exception", "reason": e.to_string()}, "status": 404})),
        ),
    }
}

async fn count(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.count(&index) {
        Ok(n) => (StatusCode::OK, Json(json!(CountResponse {
            count: n,
            shards: ShardStats::default()
        }))),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"reason": e.to_string()}, "status": 404})),
        ),
    }
}

async fn refresh_index(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
) -> Json<Value> {
    // In-memory engine has no refresh gap; this is a no-op.
    Json(json!({"_shards": {"total": 1, "successful": 1, "failed": 0}}))
}

async fn get_mapping(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.get_index_info(&index) {
        Ok(info) => (StatusCode::OK, Json(json!({index: {"mappings": info.mappings}}))),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({"error": {"reason": e.to_string()}}))),
    }
}

#[derive(Deserialize)]
struct PutMappingBody {
    properties: Option<serde_json::Map<String, Value>>,
}

async fn put_mapping(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
    Json(_body): Json<PutMappingBody>,
) -> Json<Value> {
    // Dynamic mapping update is acknowledged; a real impl would merge properties.
    Json(json!({"acknowledged": true}))
}

// ─────────────────────────────────────────────────────────────────────────────
// Document CRUD
// ─────────────────────────────────────────────────────────────────────────────

async fn index_doc_auto(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let source = flatten_object(body);
    let doc = Document::new(index.clone(), source);
    let id = doc.id.clone();

    match state.engine.index_document(&index, doc) {
        Ok(returned_id) => (StatusCode::CREATED, Json(json!({
            "_index": index,
            "_id": returned_id,
            "_version": 1,
            "result": "created",
            "_shards": {"total": 1, "successful": 1, "failed": 0}
        }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": {"type": "internal_error", "reason": e.to_string()},
            "status": 500
        }))),
    }
}

async fn index_doc_with_id(
    Path((index, id)): Path<(String, String)>,
    State(state): State<Arc<SearchState>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let source = flatten_object(body);
    let doc = Document::with_id(id.clone(), index.clone(), source);
    let is_new = state.engine.get_document(&index, &id)
        .ok()
        .and_then(|d| d)
        .is_none();

    match state.engine.index_document(&index, doc) {
        Ok(_) => {
            let status = if is_new { StatusCode::CREATED } else { StatusCode::OK };
            (status, Json(json!({
                "_index": index,
                "_id": id,
                "_version": 1,
                "result": if is_new { "created" } else { "updated" },
                "_shards": {"total": 1, "successful": 1, "failed": 0}
            })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": {"type": "internal_error", "reason": e.to_string()},
            "status": 500
        }))),
    }
}

async fn get_doc(
    Path((index, id)): Path<(String, String)>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.get_document(&index, &id) {
        Ok(Some(doc)) => (StatusCode::OK, Json(json!({
            "_index": index,
            "_id": id,
            "_version": doc.version.unwrap_or(1),
            "found": true,
            "_source": doc.source
        }))),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({
            "_index": index,
            "_id": id,
            "found": false
        }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": {"type": "internal_error", "reason": e.to_string()},
            "status": 500
        }))),
    }
}

async fn delete_doc(
    Path((index, id)): Path<(String, String)>,
    State(state): State<Arc<SearchState>>,
) -> impl IntoResponse {
    match state.engine.delete_document(&index, &id) {
        Ok(true) => (StatusCode::OK, Json(json!({
            "_index": index,
            "_id": id,
            "_version": 1,
            "result": "deleted",
            "_shards": {"total": 1, "successful": 1, "failed": 0}
        }))),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({
            "_index": index,
            "_id": id,
            "_version": 1,
            "result": "not_found",
            "_shards": {"total": 1, "successful": 0, "failed": 0}
        }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({
            "error": {"type": "internal_error", "reason": e.to_string()},
            "status": 500
        }))),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Search
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct SearchQueryParams {
    q: Option<String>,
    size: Option<usize>,
    from: Option<usize>,
    sort: Option<String>,
}

async fn search(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    run_search(index, state, req).await
}

async fn search_get(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
    Query(params): Query<SearchQueryParams>,
) -> impl IntoResponse {
    let mut req = SearchRequest::default();
    if let Some(q) = params.q {
        req.query = Some(json!({"query_string": {"query": q}}));
    }
    if let Some(sz) = params.size { req.size = sz; }
    if let Some(from) = params.from { req.from = from; }
    run_search(index, state, req).await
}

async fn run_search(
    index: String,
    state: Arc<SearchState>,
    req: SearchRequest,
) -> impl IntoResponse {
    match state.engine.search(&index, req) {
        Ok(resp) => (StatusCode::OK, Json(serde_json::to_value(resp).unwrap_or_default())),
        Err(SearchError::IndexNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"type": "index_not_found_exception", "reason": format!("no such index [{}]", index)}, "status": 404})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": {"type": "internal_error", "reason": e.to_string()}, "status": 500})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Bulk API
// ─────────────────────────────────────────────────────────────────────────────

async fn bulk(
    State(state): State<Arc<SearchState>>,
    body: String,
) -> impl IntoResponse {
    handle_bulk(None, state, body).await
}

async fn bulk_index(
    Path(index): Path<String>,
    State(state): State<Arc<SearchState>>,
    body: String,
) -> impl IntoResponse {
    handle_bulk(Some(index), state, body).await
}

async fn handle_bulk(
    default_index: Option<String>,
    state: Arc<SearchState>,
    body: String,
) -> impl IntoResponse {
    let ops = parse_bulk_body(&body);
    let response = state.engine.bulk(default_index.as_deref(), ops);
    let status = if response.errors { StatusCode::OK } else { StatusCode::OK };
    (status, Json(serde_json::to_value(&response).unwrap_or_default()))
}

/// Parse newline-delimited JSON bulk body.
fn parse_bulk_body(body: &str) -> Vec<(String, BulkActionMeta, Option<Value>)> {
    let mut ops = Vec::new();
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut i = 0;

    while i < lines.len() {
        let action_line = lines[i];
        i += 1;

        let Ok(action_obj) = serde_json::from_str::<Value>(action_line) else { continue };
        let Some(action_map) = action_obj.as_object() else { continue };

        let Some((action, meta_val)) = action_map.iter().next() else { continue };
        let meta: BulkActionMeta = serde_json::from_value(meta_val.clone()).unwrap_or(BulkActionMeta {
            index: None, id: None, routing: None,
        });

        let body = if action != "delete" && i < lines.len() {
            let body_line = lines[i];
            i += 1;
            serde_json::from_str::<Value>(body_line).ok()
        } else {
            None
        };

        ops.push((action.clone(), meta, body));
    }

    ops
}

// ─────────────────────────────────────────────────────────────────────────────
// Cat API
// ─────────────────────────────────────────────────────────────────────────────

async fn cat_indices(
    State(state): State<Arc<SearchState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let indices = state.engine.list_indices();
    let format = params.get("format").map(|s| s.as_str()).unwrap_or("json");

    let rows: Vec<CatIndexRow> = indices
        .iter()
        .map(|name| {
            let doc_count = state.engine.count(name).unwrap_or(0);
            CatIndexRow {
                health: "green".into(),
                status: "open".into(),
                index: name.clone(),
                uuid: uuid::Uuid::new_v4().to_string(),
                pri: 1,
                rep: 0,
                docs_count: doc_count,
                docs_deleted: 0,
                store_size: "0b".into(),
            }
        })
        .collect();

    Json(serde_json::to_value(&rows).unwrap_or_default())
}

async fn list_aliases(State(state): State<Arc<SearchState>>) -> Json<Value> {
    // Collect alias info from index infos.
    let indices = state.engine.list_indices();
    let mut aliases: HashMap<String, Value> = HashMap::new();

    for index in &indices {
        if let Ok(info) = state.engine.get_index_info(index) {
            for (alias, cfg) in info.aliases {
                aliases.insert(alias, json!({"aliases": {index: cfg}}));
            }
        }
    }

    Json(serde_json::to_value(aliases).unwrap_or_default())
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Ensure a JSON value is treated as a field map.
fn flatten_object(v: Value) -> HashMap<String, Value> {
    match v {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}
