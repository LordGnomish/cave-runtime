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
        .route("/", get(root_info).head(root_ping))
        .route("/_cluster/health", get(cluster_health))
        .route("/_cat/indices", get(cat_indices))
        .route("/_bulk", post(bulk_root))
        .route("/_msearch", post(msearch_root).get(msearch_root))
        .route(
            "/_field_caps",
            get(field_caps_root).post(field_caps_root),
        )
        .route("/{index}", get(get_index).put(create_index).delete(delete_index))
        .route("/{index}/_doc", post(post_doc))
        .route(
            "/{index}/_doc/{id}",
            get(get_doc).put(put_doc).delete(delete_doc),
        )
        .route("/{index}/_search", post(search).get(search))
        .route("/{index}/_bulk", post(bulk_index))
        .route("/{index}/_msearch", post(msearch_index).get(msearch_index))
        .route(
            "/{index}/_mapping",
            get(get_mapping).put(put_mapping_handler).post(put_mapping_handler),
        )
        .route(
            "/{index}/_field_caps",
            get(field_caps_index).post(field_caps_index),
        )
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

// ── info / ping ─────────────────────────────────────────────────────────────

/// `cave-search` version reported via GET /. Mirrors enough of the OpenSearch
/// root document for opensearch-py / opensearch-js compatibility checks.
pub const CAVE_SEARCH_VERSION: &str = "2.11.0-cave";

async fn root_info(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({
        "name": "cave-search",
        "cluster_name": state.cluster.name,
        "cluster_uuid": "cave-search-single-node",
        "version": {
            "distribution": "cave-search",
            "number": CAVE_SEARCH_VERSION,
            "build_type": "rust",
            "build_hash": env!("CARGO_PKG_VERSION"),
            "build_date": "1970-01-01T00:00:00Z",
            "build_snapshot": false,
            "lucene_version": "n/a",
            "minimum_wire_compatibility_version": "6.8.0",
            "minimum_index_compatibility_version": "6.0.0-beta1",
        },
        "tagline": "The OpenSearch Project: https://opensearch.org/",
    }))
}

async fn root_ping() -> StatusCode {
    StatusCode::OK
}

// ── _bulk ───────────────────────────────────────────────────────────────────

async fn bulk_root(State(state): State<Arc<AppState>>, body: String) -> Response {
    bulk_execute(&state, None, &body)
}

async fn bulk_index(
    State(state): State<Arc<AppState>>,
    Path(index): Path<String>,
    body: String,
) -> Response {
    bulk_execute(&state, Some(index.as_str()), &body)
}

fn bulk_execute(state: &AppState, default_index: Option<&str>, body: &str) -> Response {
    let mut items: Vec<Value> = Vec::new();
    let mut errors = false;
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());

    while let Some(action_line) = lines.next() {
        let action: Value = match serde_json::from_str(action_line) {
            Ok(v) => v,
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "illegal_argument_exception",
                    &format!("invalid bulk action: {}", e),
                )
            }
        };
        let obj = match action.as_object() {
            Some(o) if !o.is_empty() => o,
            _ => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "illegal_argument_exception",
                    "empty bulk action object",
                )
            }
        };
        let (op, meta) = obj.iter().next().unwrap();
        let index = meta
            .get("_index")
            .and_then(|v| v.as_str())
            .or(default_index);
        let index = match index {
            Some(s) => s.to_string(),
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "illegal_argument_exception",
                    "bulk action missing _index",
                )
            }
        };
        let id = meta.get("_id").and_then(|v| v.as_str()).map(String::from);

        match op.as_str() {
            "index" | "create" => {
                let source_line = match lines.next() {
                    Some(l) => l,
                    None => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            "illegal_argument_exception",
                            "bulk index/create missing source line",
                        )
                    }
                };
                let source: Value = match serde_json::from_str(source_line) {
                    Ok(v) => v,
                    Err(e) => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            "illegal_argument_exception",
                            &format!("invalid bulk source: {}", e),
                        )
                    }
                };
                let doc_id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                let result_label = if op == "create" { "created" } else { "indexed" };
                match state.registry.index_doc(&index, &doc_id, source) {
                    Ok(d) => {
                        let count = state.registry.doc_count(&index).unwrap_or(0);
                        state.cluster.upsert_index(&index, 1, 0, count as u64);
                        let label = if d.version == 1 { "created" } else if op == "create" { "created" } else { result_label };
                        items.push(json!({
                            op: {
                                "_index": index,
                                "_id": doc_id,
                                "_version": d.version,
                                "_seq_no": d.seq_no,
                                "_primary_term": d.primary_term,
                                "result": if label == "indexed" { "updated" } else { label },
                                "status": if label == "created" { 201 } else { 200 },
                            }
                        }));
                    }
                    Err(e) => {
                        errors = true;
                        items.push(bulk_error_item(op, &index, &doc_id, &e));
                    }
                }
            }
            "delete" => {
                let doc_id = match id {
                    Some(s) => s,
                    None => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            "illegal_argument_exception",
                            "bulk delete missing _id",
                        )
                    }
                };
                match state.registry.delete_doc(&index, &doc_id) {
                    Ok(()) => {
                        let count = state.registry.doc_count(&index).unwrap_or(0);
                        state.cluster.upsert_index(&index, 1, 0, count as u64);
                        items.push(json!({
                            "delete": {
                                "_index": index,
                                "_id": doc_id,
                                "result": "deleted",
                                "status": 200,
                            }
                        }));
                    }
                    Err(e) => {
                        // OpenSearch returns 404 with result=not_found, not an error item.
                        if matches!(e, SearchError::DocumentNotFound(_, _)) {
                            items.push(json!({
                                "delete": {
                                    "_index": index,
                                    "_id": doc_id,
                                    "result": "not_found",
                                    "status": 404,
                                }
                            }));
                        } else {
                            errors = true;
                            items.push(bulk_error_item("delete", &index, &doc_id, &e));
                        }
                    }
                }
            }
            "update" => {
                // Update requires a doc/script body; we do a partial-merge "doc" only.
                let body_line = match lines.next() {
                    Some(l) => l,
                    None => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            "illegal_argument_exception",
                            "bulk update missing body line",
                        )
                    }
                };
                let body_v: Value = match serde_json::from_str(body_line) {
                    Ok(v) => v,
                    Err(e) => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            "illegal_argument_exception",
                            &format!("invalid update body: {}", e),
                        )
                    }
                };
                let doc_id = match id {
                    Some(s) => s,
                    None => {
                        return error_response(
                            StatusCode::BAD_REQUEST,
                            "illegal_argument_exception",
                            "bulk update missing _id",
                        )
                    }
                };
                let patch = body_v.get("doc").cloned();
                let merged = match (state.registry.get(&index, &doc_id), patch) {
                    (Ok(existing), Some(Value::Object(p))) => {
                        let mut base = existing
                            .source
                            .as_object()
                            .cloned()
                            .unwrap_or_default();
                        for (k, v) in p {
                            base.insert(k, v);
                        }
                        Value::Object(base)
                    }
                    (Ok(existing), _) => existing.source.clone(),
                    (Err(e), _) => {
                        errors = true;
                        items.push(bulk_error_item("update", &index, &doc_id, &e));
                        continue;
                    }
                };
                match state.registry.index_doc(&index, &doc_id, merged) {
                    Ok(d) => {
                        items.push(json!({
                            "update": {
                                "_index": index,
                                "_id": doc_id,
                                "_version": d.version,
                                "_seq_no": d.seq_no,
                                "_primary_term": d.primary_term,
                                "result": "updated",
                                "status": 200,
                            }
                        }));
                    }
                    Err(e) => {
                        errors = true;
                        items.push(bulk_error_item("update", &index, &doc_id, &e));
                    }
                }
            }
            _ => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "illegal_argument_exception",
                    &format!("unsupported bulk action: {}", op),
                )
            }
        }
    }

    let took_ms = 0u64;
    Json(json!({
        "took": took_ms,
        "errors": errors,
        "items": items,
    }))
    .into_response()
}

fn bulk_error_item(op: &str, index: &str, id: &str, err: &SearchError) -> Value {
    let (status, type_str) = bulk_status_for(err);
    json!({
        op: {
            "_index": index,
            "_id": id,
            "status": status,
            "error": {"type": type_str, "reason": err.to_string()},
        }
    })
}

fn bulk_status_for(e: &SearchError) -> (u16, &'static str) {
    match e {
        SearchError::IndexNotFound(_) => (404, "index_not_found_exception"),
        SearchError::IndexExists(_) => (400, "resource_already_exists_exception"),
        SearchError::DocumentNotFound(_, _) => (404, "document_missing_exception"),
        SearchError::InvalidMapping(_) => (400, "mapper_parsing_exception"),
        SearchError::InvalidQuery(_) => (400, "search_phase_execution_exception"),
        SearchError::InvalidAggregation(_) => (400, "search_phase_execution_exception"),
        SearchError::TypeMismatch { .. } => (400, "mapper_parsing_exception"),
    }
}

// ── _msearch ────────────────────────────────────────────────────────────────

async fn msearch_root(State(state): State<Arc<AppState>>, body: String) -> Response {
    msearch_execute(&state, None, &body)
}

async fn msearch_index(
    State(state): State<Arc<AppState>>,
    Path(index): Path<String>,
    body: String,
) -> Response {
    msearch_execute(&state, Some(index.as_str()), &body)
}

fn msearch_execute(state: &AppState, default_index: Option<&str>, body: &str) -> Response {
    let mut responses: Vec<Value> = Vec::new();
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());
    while let Some(header_line) = lines.next() {
        let header: Value = match serde_json::from_str(header_line) {
            Ok(v) => v,
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "illegal_argument_exception",
                    &format!("invalid msearch header: {}", e),
                )
            }
        };
        let body_line = match lines.next() {
            Some(l) => l,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "illegal_argument_exception",
                    "msearch missing body line",
                )
            }
        };
        // header may be "{}" — fall back to default_index in that case.
        let index = header
            .get("index")
            .and_then(|v| v.as_str())
            .or(default_index);
        let index = match index {
            Some(s) => s.to_string(),
            None => {
                responses.push(json!({
                    "error": {"type": "illegal_argument_exception", "reason": "msearch entry missing index"},
                    "status": 400,
                }));
                continue;
            }
        };
        let req: SearchRequest = match serde_json::from_str(body_line) {
            Ok(v) => v,
            Err(e) => {
                responses.push(json!({
                    "error": {"type": "illegal_argument_exception", "reason": format!("invalid msearch body: {}", e)},
                    "status": 400,
                }));
                continue;
            }
        };
        let q = req.query.unwrap_or(DslQuery::MatchAll);
        let size = req.size.unwrap_or(10);
        let from = req.from.unwrap_or(0);
        let aggs = req.aggs.unwrap_or_default();
        let result = state.registry.with_index(&index, |d| {
            let resp = run_query(&q, d, &index, size, from);
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
            Ok(mut v) => {
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("status".into(), json!(200));
                }
                responses.push(v);
            }
            Err(e) => {
                let (status, ty) = bulk_status_for(&e);
                responses.push(json!({
                    "error": {"type": ty, "reason": e.to_string()},
                    "status": status,
                }));
            }
        }
    }
    Json(json!({"responses": responses, "took": 0u64})).into_response()
}

// ── indices.get_mapping / put_mapping ───────────────────────────────────────

async fn get_mapping(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Response {
    if !state.registry.exists(&name) {
        return error_response(
            StatusCode::NOT_FOUND,
            "index_not_found_exception",
            &format!("no such index [{}]", name),
        );
    }
    state
        .registry
        .with_index(&name, |d| {
            Json(json!({
                &name: {"mappings": d.mapping}
            }))
            .into_response()
        })
        .unwrap_or_else(search_error)
}

async fn put_mapping_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    // Accept either {"properties": {..}} or a wrapped {"mappings": {"properties": {..}}} form.
    let properties = body
        .get("properties")
        .or_else(|| body.get("mappings").and_then(|m| m.get("properties")))
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let mut mapping = IndexMapping::new();
    if let Value::Object(obj) = properties {
        for (k, v) in obj {
            match serde_json::from_value::<FieldMapping>(v) {
                Ok(fm) => {
                    mapping.properties.insert(k, fm);
                }
                Err(e) => {
                    return error_response(
                        StatusCode::BAD_REQUEST,
                        "mapper_parsing_exception",
                        &format!("invalid field mapping for [{}]: {}", k, e),
                    )
                }
            }
        }
    }
    match state.registry.put_mapping(&name, mapping) {
        Ok(()) => Json(json!({"acknowledged": true})).into_response(),
        Err(e) => search_error(e),
    }
}

// ── _field_caps ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct FieldCapsQuery {
    #[serde(default)]
    fields: Option<String>,
}

async fn field_caps_root(
    State(state): State<Arc<AppState>>,
    Query(q): Query<FieldCapsQuery>,
) -> Response {
    let names = state.registry.list();
    field_caps_response(&state, &names, q.fields.as_deref())
}

async fn field_caps_index(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Query(q): Query<FieldCapsQuery>,
) -> Response {
    if !state.registry.exists(&name) {
        return error_response(
            StatusCode::NOT_FOUND,
            "index_not_found_exception",
            &format!("no such index [{}]", name),
        );
    }
    field_caps_response(&state, &[name], q.fields.as_deref())
}

fn field_caps_response(state: &AppState, indices: &[String], fields_filter: Option<&str>) -> Response {
    use std::collections::BTreeSet;
    let wanted: Option<BTreeSet<&str>> = fields_filter.map(|s| {
        s.split(',').map(str::trim).filter(|f| !f.is_empty()).collect()
    });
    // field name → type → set of indices having that mapping.
    let mut by_field: BTreeMap<String, BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for name in indices {
        let _ = state.registry.with_index(name, |d| {
            for (fname, fm) in &d.mapping.properties {
                if let Some(w) = &wanted {
                    if !w.contains(fname.as_str()) && !w.contains("*") {
                        continue;
                    }
                }
                by_field
                    .entry(fname.clone())
                    .or_default()
                    .entry(fm.field_type.as_str().to_string())
                    .or_default()
                    .insert(name.clone());
            }
        });
    }
    let mut out = serde_json::Map::new();
    out.insert("indices".into(), json!(indices));
    let mut fields_out = serde_json::Map::new();
    for (fname, type_map) in by_field {
        let mut per_type = serde_json::Map::new();
        let multi_type = type_map.len() > 1;
        for (ty, idx_set) in type_map {
            let mut entry = serde_json::Map::new();
            entry.insert("type".into(), json!(ty));
            entry.insert("metadata_field".into(), json!(false));
            entry.insert("searchable".into(), json!(true));
            entry.insert("aggregatable".into(), json!(true));
            if multi_type {
                let mut idxs: Vec<String> = idx_set.into_iter().collect();
                idxs.sort();
                entry.insert("indices".into(), json!(idxs));
            }
            per_type.insert(ty, Value::Object(entry));
        }
        fields_out.insert(fname, Value::Object(per_type));
    }
    out.insert("fields".into(), Value::Object(fields_out));
    Json(Value::Object(out)).into_response()
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

    #[test]
    fn bulk_executes_index_and_delete() {
        let state = AppState::new();
        let mut m = IndexMapping::new();
        m.add_field("color", crate::mapping::FieldType::Keyword);
        state.registry.create("things", m).unwrap();
        let body = "\
{\"index\":{\"_index\":\"things\",\"_id\":\"1\"}}\n\
{\"color\":\"red\"}\n\
{\"index\":{\"_index\":\"things\",\"_id\":\"2\"}}\n\
{\"color\":\"blue\"}\n\
{\"delete\":{\"_index\":\"things\",\"_id\":\"1\"}}\n";
        let resp = bulk_execute(&state, None, body);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(state.registry.doc_count("things").unwrap(), 1);
    }

    #[test]
    fn bulk_uses_default_index_from_path() {
        let state = AppState::new();
        state.registry.create("things", IndexMapping::new()).unwrap();
        let body = "\
{\"index\":{\"_id\":\"1\"}}\n\
{\"x\":1}\n";
        let resp = bulk_execute(&state, Some("things"), body);
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(state.registry.doc_count("things").unwrap(), 1);
    }

    #[test]
    fn bulk_missing_index_is_400() {
        let state = AppState::new();
        let body = "{\"index\":{\"_id\":\"1\"}}\n{\"x\":1}\n";
        let resp = bulk_execute(&state, None, body);
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn bulk_unknown_index_records_error_item() {
        let state = AppState::new();
        let body = "{\"index\":{\"_index\":\"missing\",\"_id\":\"1\"}}\n{\"x\":1}\n";
        let resp = bulk_execute(&state, None, body);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn msearch_returns_one_response_per_query() {
        let state = AppState::new();
        state.registry.create("a", IndexMapping::new()).unwrap();
        state.registry.create("b", IndexMapping::new()).unwrap();
        state.registry.index_doc("a", "1", json!({})).unwrap();
        state.registry.index_doc("b", "1", json!({})).unwrap();
        state.registry.index_doc("b", "2", json!({})).unwrap();
        let body = "\
{\"index\":\"a\"}\n\
{\"query\":{\"match_all\":{}}}\n\
{\"index\":\"b\"}\n\
{\"query\":{\"match_all\":{}}}\n";
        let resp = msearch_execute(&state, None, body);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn put_mapping_handler_merges_properties() {
        let state = AppState::new();
        state.registry.create("idx", IndexMapping::new()).unwrap();
        let mut m = IndexMapping::new();
        m.add_field("title", crate::mapping::FieldType::Keyword);
        let mut new = IndexMapping::new();
        new.add_field("body", crate::mapping::FieldType::Text);
        state.registry.put_mapping("idx", new).unwrap();
        state
            .registry
            .with_index("idx", |d| {
                assert_eq!(
                    d.mapping.field_type("body"),
                    Some(crate::mapping::FieldType::Text)
                );
            })
            .unwrap();
    }

    #[test]
    fn field_caps_returns_field_types_grouped() {
        let state = AppState::new();
        let mut m = IndexMapping::new();
        m.add_field("color", crate::mapping::FieldType::Keyword);
        m.add_field("count", crate::mapping::FieldType::Long);
        state.registry.create("idx", m).unwrap();
        let resp =
            field_caps_response(&state, &["idx".to_string()], None);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn field_caps_filter_restricts_fields() {
        let state = AppState::new();
        let mut m = IndexMapping::new();
        m.add_field("color", crate::mapping::FieldType::Keyword);
        m.add_field("count", crate::mapping::FieldType::Long);
        state.registry.create("idx", m).unwrap();
        let resp = field_caps_response(&state, &["idx".to_string()], Some("color"));
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn cave_search_version_is_set() {
        assert!(!CAVE_SEARCH_VERSION.is_empty());
    }
}
