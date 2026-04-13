//! HTTP route handlers for cave-vector-search.
//!
//! Provides a Qdrant-compatible REST API:
//!
//! | Method | Path                                                       | Description                   |
//! |--------|------------------------------------------------------------|-------------------------------|
//! | GET    | /api/vectors/health                                        | Health check                  |
//! | GET    | /api/vectors/collections                                   | List collections              |
//! | PUT    | /api/vectors/collections/{name}                            | Create collection             |
//! | DELETE | /api/vectors/collections/{name}                            | Delete collection             |
//! | GET    | /api/vectors/collections/{name}                            | Get collection info           |
//! | POST   | /api/vectors/collections/{name}/points                     | Upsert points                 |
//! | GET    | /api/vectors/collections/{name}/points/{id}                | Get single point              |
//! | POST   | /api/vectors/collections/{name}/points/get                 | Get multiple points           |
//! | POST   | /api/vectors/collections/{name}/points/delete              | Delete points                 |
//! | PUT    | /api/vectors/collections/{name}/points/payload             | Set payload                   |
//! | POST   | /api/vectors/collections/{name}/points/search              | Vector search                 |
//! | POST   | /api/vectors/collections/{name}/points/scroll              | Scroll all points             |
//! | POST   | /api/vectors/collections/{name}/points/recommend           | Recommend by example          |
//! | PUT    | /api/vectors/collections/{name}/index                      | Create payload field index    |

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
    CollectionConfig, CreateFieldIndexRequest, DeletePointsRequest, Distance, Filter, HnswConfig,
    Point, PointId, RecommendRequest, ScrollRequest, SearchRequest, SetPayloadRequest,
    UpsertPointsRequest, VectorParams,
};
use crate::{VectorError, VectorState};

// ─────────────────────────────────────────────────────────────────────────────
// Router factory
// ─────────────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<VectorState>) -> Router {
    Router::new()
        // Health
        .route("/api/vectors/health", get(health))
        // Collections
        .route("/api/vectors/collections", get(list_collections))
        .route("/api/vectors/collections/{name}", put(create_collection))
        .route("/api/vectors/collections/{name}", delete(delete_collection))
        .route("/api/vectors/collections/{name}", get(get_collection))
        // Points — batch ops first to avoid path conflicts
        .route("/api/vectors/collections/{name}/points", post(upsert_points))
        .route("/api/vectors/collections/{name}/points/get", post(get_points_batch))
        .route("/api/vectors/collections/{name}/points/delete", post(delete_points))
        .route("/api/vectors/collections/{name}/points/payload", put(set_payload))
        .route("/api/vectors/collections/{name}/points/search", post(search))
        .route("/api/vectors/collections/{name}/points/scroll", post(scroll))
        .route("/api/vectors/collections/{name}/points/recommend", post(recommend))
        .route("/api/vectors/collections/{name}/points/{id}", get(get_point))
        // Payload field index
        .route("/api/vectors/collections/{name}/index", put(create_field_index))
        .with_state(state)
}

// ─────────────────────────────────────────────────────────────────────────────
// Health
// ─────────────────────────────────────────────────────────────────────────────

async fn health(State(state): State<Arc<VectorState>>) -> Json<Value> {
    let count = state.store.list_collections().len();
    Json(json!({
        "module": "cave-vector-search",
        "status": "ok",
        "upstream": "Qdrant",
        "implementation": "built-in HNSW index",
        "collections": count,
        "features": [
            "hnsw-index",
            "cosine-similarity",
            "euclidean-distance",
            "dot-product",
            "manhattan-distance",
            "payload-filtering",
            "payload-indexing",
            "scroll-api",
            "recommend-api",
            "bulk-upsert",
            "tenant-isolation"
        ]
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// Collection management
// ─────────────────────────────────────────────────────────────────────────────

async fn list_collections(State(state): State<Arc<VectorState>>) -> Json<Value> {
    let collections = state.store.list_collections();
    let items: Vec<Value> = collections
        .iter()
        .map(|name| json!({"name": name}))
        .collect();
    Json(json!({
        "result": {"collections": items},
        "status": "ok",
        "time": 0.0
    }))
}

#[derive(Deserialize)]
struct CreateCollectionBody {
    vectors: VectorParamsBody,
    #[serde(default)]
    hnsw_config: Option<HnswConfigBody>,
    #[serde(default)]
    replication_factor: Option<u32>,
}

#[derive(Deserialize)]
struct VectorParamsBody {
    size: usize,
    #[serde(default)]
    distance: DistanceBody,
    #[serde(default)]
    on_disk: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
enum DistanceBody {
    #[default]
    Cosine,
    Euclid,
    Dot,
    Manhattan,
}

impl From<DistanceBody> for Distance {
    fn from(d: DistanceBody) -> Self {
        match d {
            DistanceBody::Cosine => Distance::Cosine,
            DistanceBody::Euclid => Distance::Euclid,
            DistanceBody::Dot => Distance::Dot,
            DistanceBody::Manhattan => Distance::Manhattan,
        }
    }
}

#[derive(Deserialize, Default)]
struct HnswConfigBody {
    #[serde(default)]
    m: Option<usize>,
    #[serde(default)]
    ef_construction: Option<usize>,
    #[serde(default)]
    ef: Option<usize>,
}

async fn create_collection(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    body: Option<Json<CreateCollectionBody>>,
) -> impl IntoResponse {
    let (dim, distance, hnsw_config) = if let Some(Json(b)) = body {
        let hnsw = HnswConfig {
            m: b.hnsw_config.as_ref().and_then(|h| h.m).unwrap_or(16),
            m0: b.hnsw_config.as_ref().and_then(|h| h.m).map(|m| m * 2).unwrap_or(32),
            ef_construction: b.hnsw_config.as_ref().and_then(|h| h.ef_construction).unwrap_or(100),
            ef: b.hnsw_config.as_ref().and_then(|h| h.ef).unwrap_or(64),
        };
        (b.vectors.size, Distance::from(b.vectors.distance), hnsw)
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"status": "error", "error": "request body required"})),
        );
    };

    let mut config = CollectionConfig::new(dim, distance);
    config.hnsw_config = hnsw_config;

    match state.store.create_collection(&name, config) {
        Ok(()) => {
            info!(collection = %name, "collection created");
            (StatusCode::OK, Json(json!({"result": true, "status": "ok", "time": 0.0})))
        }
        Err(VectorError::CollectionAlreadyExists(_)) => (
            StatusCode::CONFLICT,
            Json(json!({"status": "error", "error": format!("collection {} already exists", name)})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

async fn delete_collection(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
) -> impl IntoResponse {
    match state.store.delete_collection(&name) {
        Ok(()) => (StatusCode::OK, Json(json!({"result": true, "status": "ok", "time": 0.0}))),
        Err(VectorError::CollectionNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": "error", "error": format!("collection {} not found", name)})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

async fn get_collection(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
) -> impl IntoResponse {
    match state.store.collection_info(&name) {
        Ok(info) => (StatusCode::OK, Json(json!({"result": info, "status": "ok", "time": 0.0}))),
        Err(VectorError::CollectionNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": "error", "error": format!("collection {} not found", name)})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Points — upsert
// ─────────────────────────────────────────────────────────────────────────────

async fn upsert_points(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<UpsertPointsRequest>,
) -> impl IntoResponse {
    let points = if let Some(batch) = req.batch {
        // Convert parallel-arrays batch format to individual points.
        batch
            .ids
            .into_iter()
            .zip(batch.vectors.into_iter())
            .enumerate()
            .map(|(i, (id, vec))| {
                let payload = batch.payloads.get(i).cloned().unwrap_or_default();
                Point::new(id, vec, payload)
            })
            .collect()
    } else {
        req.points
    };

    match state.store.upsert_points(&name, points) {
        Ok(result) => (StatusCode::OK, Json(json!({
            "result": result,
            "status": "ok",
            "time": 0.0
        }))),
        Err(VectorError::CollectionNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": "error", "error": format!("collection {} not found", name)})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Points — get
// ─────────────────────────────────────────────────────────────────────────────

async fn get_point(
    Path((name, id)): Path<(String, String)>,
    State(state): State<Arc<VectorState>>,
) -> impl IntoResponse {
    let point_id = parse_point_id(&id);
    match state.store.get_point(&name, &point_id) {
        Ok(Some(p)) => (StatusCode::OK, Json(json!({"result": p, "status": "ok", "time": 0.0}))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": "error", "error": format!("point {} not found", id)})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

#[derive(Deserialize)]
struct GetPointsBatchRequest {
    ids: Vec<PointId>,
    #[serde(default = "default_true")]
    with_payload: bool,
    #[serde(default)]
    with_vectors: bool,
}
fn default_true() -> bool { true }

async fn get_points_batch(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<GetPointsBatchRequest>,
) -> impl IntoResponse {
    match state.store.get_points(&name, &req.ids, req.with_payload, req.with_vectors) {
        Ok(points) => (StatusCode::OK, Json(json!({"result": points, "status": "ok", "time": 0.0}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Points — delete
// ─────────────────────────────────────────────────────────────────────────────

async fn delete_points(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<DeletePointsRequest>,
) -> impl IntoResponse {
    match state.store.delete_points(&name, req.points, req.filter) {
        Ok(result) => (StatusCode::OK, Json(json!({
            "result": result,
            "status": "ok",
            "time": 0.0
        }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload
// ─────────────────────────────────────────────────────────────────────────────

async fn set_payload(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<SetPayloadRequest>,
) -> impl IntoResponse {
    match state.store.set_payload(&name, req) {
        Ok(result) => (StatusCode::OK, Json(json!({"result": result, "status": "ok", "time": 0.0}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Search
// ─────────────────────────────────────────────────────────────────────────────

async fn search(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    match state.store.search(&name, req) {
        Ok(results) => (StatusCode::OK, Json(json!({
            "result": results,
            "status": "ok",
            "time": 0.0
        }))),
        Err(VectorError::CollectionNotFound(_)) => (
            StatusCode::NOT_FOUND,
            Json(json!({"status": "error", "error": format!("collection {} not found", name)})),
        ),
        Err(VectorError::DimensionMismatch { expected, got }) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"status": "error", "error": format!("dimension mismatch: expected {}, got {}", expected, got)})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scroll
// ─────────────────────────────────────────────────────────────────────────────

async fn scroll(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<ScrollRequest>,
) -> impl IntoResponse {
    match state.store.scroll(
        &name,
        req.filter,
        req.limit,
        req.offset,
        req.with_payload,
        req.with_vectors,
    ) {
        Ok(resp) => (StatusCode::OK, Json(json!({
            "result": resp,
            "status": "ok",
            "time": 0.0
        }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recommend
// ─────────────────────────────────────────────────────────────────────────────

async fn recommend(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<RecommendRequest>,
) -> impl IntoResponse {
    match state.store.recommend(&name, req) {
        Ok(results) => (StatusCode::OK, Json(json!({
            "result": results,
            "status": "ok",
            "time": 0.0
        }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Payload field index
// ─────────────────────────────────────────────────────────────────────────────

async fn create_field_index(
    Path(name): Path<String>,
    State(state): State<Arc<VectorState>>,
    Json(req): Json<CreateFieldIndexRequest>,
) -> impl IntoResponse {
    match state.store.create_field_index(&name, &req.field_name, req.field_schema) {
        Ok(result) => (StatusCode::OK, Json(json!({"result": result, "status": "ok", "time": 0.0}))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"status": "error", "error": e.to_string()})),
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn parse_point_id(s: &str) -> PointId {
    if let Ok(n) = s.parse::<u64>() {
        PointId::Num(n)
    } else {
        PointId::Uuid(s.to_string())
    }
}
