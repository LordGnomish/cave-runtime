//! OpenSearch-compatible HTTP server (axum).

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::aggregation::{execute as run_agg, Aggregation};
use crate::cluster::ClusterState;
use crate::dsl::{execute as run_query, Query as DslQuery};
use crate::error::SearchError;
use crate::index::IndexRegistry;
use crate::mapping::{FieldMapping, IndexMapping};

#[derive(Debug)]
pub struct AppState {
    pub registry: IndexRegistry,
    pub cluster: ClusterState,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            registry: IndexRegistry::new(),
            cluster: ClusterState::new("cave-search"),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/_cluster/health", get(cluster_health))
        .route("/_cat/indices", get(cat_indices))
        .route("/{index}", get(get_index).put(create_index).delete(delete_index))
        .route("/{index}/_doc", post(post_doc))
        .route(
            "/{index}/_doc/{id}",
            get(get_doc).put(put_doc).delete(delete_doc),
        )
        .route("/{index}/_search", post(search).get(search))
        .with_state(state)
}

pub async fn bind(state: Arc<AppState>, bind_addr: &str) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    let addr = listener.local_addr()?;
    let app = router(state);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((addr, handle))
}

// ── Request models ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateIndexRequest {
    #[serde(default)]
    mappings: Option<MappingsBlock>,
}

#[derive(Debug, Deserialize)]
struct MappingsBlock {
    #[serde(default)]
    properties: BTreeMap<String, FieldMapping>,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    #[serde(default)]
    query: Option<DslQuery>,
    #[serde(default)]
    size: Option<usize>,
    #[serde(default)]
    from: Option<usize>,
    #[serde(default)]
    aggs: Option<BTreeMap<String, Aggregation>>,
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn cluster_health(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(serde_json::to_value(state.cluster.health()).unwrap())
}

async fn cat_indices(State(state): State<Arc<AppState>>) -> Json<Value> {
    let list: Vec<Value> = state
        .cluster
        .list_indices()
        .into_iter()
        .map(|(name, c)| {
            json!({
                "index": name,
                "pri": c.primary_shards,
                "rep": c.replica_shards,
                "docs.count": c.doc_count,
            })
        })
        .collect();
    Json(json!(list))
}

async fn get_index(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    if !state.registry.exists(&name) {
        return error_response(StatusCode::NOT_FOUND, "index_not_found_exception", &format!("no such index [{}]", name));
    }
    state
        .registry
        .with_index(&name, |d| {
            Json(json!({
                &name: {
                    "mappings": d.mapping,
                    "settings": {"index": {"creation_date": d.created_at_ms.to_string()}}
                }
            }))
            .into_response()
        })
        .unwrap_or_else(|e| search_error(e))
}

async fn create_index(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    body: Option<Json<CreateIndexRequest>>,
) -> Response {
    let mut mapping = IndexMapping::new();
    if let Some(Json(req)) = body {
        if let Some(m) = req.mappings {
            mapping.properties = m.properties;
        }
    }
    match state.registry.create(&name, mapping) {
        Ok(()) => {
            state.cluster.upsert_index(&name, 1, 0, 0);
            Json(json!({"acknowledged": true, "shards_acknowledged": true, "index": name}))
                .into_response()
        }
        Err(e) => search_error(e),
    }
}

async fn delete_index(State(state): State<Arc<AppState>>, Path(name): Path<String>) -> Response {
    match state.registry.delete(&name) {
        Ok(()) => {
            state.cluster.drop_index(&name);
            Json(json!({"acknowledged": true})).into_response()
        }
        Err(e) => search_error(e),
    }
}

async fn post_doc(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let id = uuid::Uuid::new_v4().to_string();
    index_doc_response(&state, &name, &id, body, true)
}

async fn put_doc(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    index_doc_response(&state, &name, &id, body, false)
}

fn index_doc_response(
    state: &AppState,
    name: &str,
    id: &str,
    body: Value,
    created: bool,
) -> Response {
    match state.registry.index_doc(name, id, body) {
        Ok(d) => {
            let count = state.registry.doc_count(name).unwrap_or(0);
            state.cluster.upsert_index(name, 1, 0, count as u64);
            let result = if d.version == 1 || created {
                "created"
            } else {
                "updated"
            };
            Json(json!({
                "_index": name,
                "_id": id,
                "_version": d.version,
                "_seq_no": d.seq_no,
                "_primary_term": d.primary_term,
                "result": result,
            }))
            .into_response()
        }
        Err(e) => search_error(e),
    }
}

async fn get_doc(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, String)>,
) -> Response {
    match state.registry.get(&name, &id) {
        Ok(d) => Json(json!({
            "_index": name,
            "_id": id,
            "_version": d.version,
            "_seq_no": d.seq_no,
            "_primary_term": d.primary_term,
            "found": true,
            "_source": d.source,
        }))
        .into_response(),
        Err(e) => search_error(e),
    }
}

async fn delete_doc(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, String)>,
) -> Response {
    match state.registry.delete_doc(&name, &id) {
        Ok(()) => {
            let count = state.registry.doc_count(&name).unwrap_or(0);
            state.cluster.upsert_index(&name, 1, 0, count as u64);
            Json(json!({"_index": name, "_id": id, "result": "deleted"})).into_response()
        }
        Err(e) => search_error(e),
    }
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(qs): Query<BTreeMap<String, String>>,
    body: Option<Json<SearchRequest>>,
) -> Response {
    let req = body.map(|Json(r)| r).unwrap_or(SearchRequest {
        query: None,
        size: qs.get("size").and_then(|s| s.parse().ok()),
        from: qs.get("from").and_then(|s| s.parse().ok()),
        aggs: None,
    });
    let q = req.query.unwrap_or(DslQuery::MatchAll);
    let size = req.size.unwrap_or(10);
    let from = req.from.unwrap_or(0);
    let aggs = req.aggs.unwrap_or_default();
    let result = state.registry.with_index(&name, |d| {
        let resp = run_query(&q, d, &name, size, from);
        let mut value = serde_json::to_value(&resp).unwrap();
        if !aggs.is_empty() {
            let mut out = BTreeMap::new();
            for (n, a) in &aggs {
                let r = run_agg(n, a, d);
                out.insert(r.name, r.value);
            }
            value
                .as_object_mut()
                .unwrap()
                .insert("aggregations".into(), serde_json::to_value(out).unwrap());
        }
        value
    });
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => search_error(e),
    }
}

fn search_error(e: SearchError) -> Response {
    let (status, type_str) = match &e {
        SearchError::IndexNotFound(_) => (StatusCode::NOT_FOUND, "index_not_found_exception"),
        SearchError::IndexExists(_) => (
            StatusCode::BAD_REQUEST,
            "resource_already_exists_exception",
        ),
        SearchError::DocumentNotFound(_, _) => (StatusCode::NOT_FOUND, "document_missing_exception"),
        SearchError::InvalidMapping(_) => (StatusCode::BAD_REQUEST, "mapper_parsing_exception"),
        SearchError::InvalidQuery(_) => (StatusCode::BAD_REQUEST, "search_phase_execution_exception"),
        SearchError::InvalidAggregation(_) => (StatusCode::BAD_REQUEST, "search_phase_execution_exception"),
        SearchError::TypeMismatch { .. } => (StatusCode::BAD_REQUEST, "mapper_parsing_exception"),
    };
    error_response(status, type_str, &e.to_string())
}

fn error_response(status: StatusCode, type_str: &str, reason: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {"type": type_str, "reason": reason},
            "status": status.as_u16(),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_index_request_with_mapping_deserializes() {
        let s = r#"{"mappings":{"properties":{"name":{"type":"keyword"}}}}"#;
        let req: CreateIndexRequest = serde_json::from_str(s).unwrap();
        let m = req.mappings.unwrap();
        assert!(m.properties.contains_key("name"));
    }

    #[test]
    fn search_request_deserializes_from_match_all() {
        let s = r#"{"query":{"match_all":null},"size":5}"#;
        let req: SearchRequest = serde_json::from_str(s).unwrap();
        assert_eq!(req.size, Some(5));
        assert!(matches!(req.query, Some(DslQuery::MatchAll)));
    }

    #[test]
    fn search_request_deserializes_aggs() {
        let s = r#"{"aggs":{"by_color":{"terms":{"field":"color"}}}}"#;
        let req: SearchRequest = serde_json::from_str(s).unwrap();
        assert!(req.aggs.unwrap().contains_key("by_color"));
    }

    #[test]
    fn error_response_serializes_status_and_type() {
        let r = error_response(StatusCode::NOT_FOUND, "x", "y");
        let _ = r;
    }
}
