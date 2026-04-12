<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/jovial-faraday
//! HTTP routes for cave-store.

use crate::models::{AccessPolicy, LifecycleRule, ReplicationRule, StorageObject};
use crate::StoreState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<StoreState>) -> Router {
    Router::new()
        // Bucket CRUD
        .route("/api/v1/store/buckets", get(list_buckets).post(create_bucket))
        .route(
            "/api/v1/store/buckets/:bucket",
            get(get_bucket).delete(delete_bucket),
        )
        // Bucket configuration
        .route(
            "/api/v1/store/buckets/:bucket/versioning",
            put(set_versioning),
        )
        .route(
            "/api/v1/store/buckets/:bucket/lifecycle",
            get(get_lifecycle).put(put_lifecycle),
        )
        .route(
            "/api/v1/store/buckets/:bucket/policy",
            get(get_policy).put(put_policy),
        )
        .route(
            "/api/v1/store/buckets/:bucket/replication",
            get(get_replication).put(put_replication),
        )
        // Object CRUD
        .route(
            "/api/v1/store/buckets/:bucket/objects",
            get(list_objects),
        )
        .route("/api/v1/store/objects", post(put_object))
        .route(
            "/api/v1/store/objects/:bucket/:key",
            get(get_object).delete(delete_object),
        )
        .route(
            "/api/v1/store/objects/:bucket/:key/versions",
            get(list_versions),
        )
        // Multipart upload
        .route("/api/v1/store/multipart/initiate", post(initiate_multipart))
        .route(
            "/api/v1/store/multipart/:upload_id/parts",
            put(upload_part),
        )
        .route(
            "/api/v1/store/multipart/:upload_id/complete",
            post(complete_multipart),
        )
        .route(
            "/api/v1/store/multipart/:upload_id",
            delete(abort_multipart),
        )
        .route(
            "/api/v1/store/buckets/:bucket/multipart",
            get(list_multipart),
        )
        .with_state(state)
}

// ── Request types ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateBucketRequest {
    name: String,
    region: Option<String>,
    tags: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct SetVersioningRequest {
    enabled: bool,
}

#[derive(Deserialize)]
struct LifecycleRuleInput {
    prefix: String,
    expiration_days: Option<u32>,
    transition_storage_class: Option<String>,
    enabled: bool,
}

#[derive(Deserialize)]
struct PutLifecycleRequest {
    rules: Vec<LifecycleRuleInput>,
}

#[derive(Deserialize)]
struct ReplicationRuleInput {
    destination_bucket: String,
    prefix: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct PutReplicationRequest {
    rules: Vec<ReplicationRuleInput>,
<<<<<<< HEAD
=======
use std::collections::HashMap;
use std::sync::Arc;
use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, head, post, put},
    Json,
};
use serde::Deserialize;
use crate::store::ObjectStore;
use crate::types::BucketPolicy;

pub type StoreState = Arc<ObjectStore>;

pub fn store_router(state: StoreState) -> Router {
    Router::new()
        .route("/api/store/health", get(health))
        .route("/api/store/buckets", get(list_buckets))
        .route("/api/store/buckets/:bucket", put(create_bucket))
        .route("/api/store/buckets/:bucket", delete(delete_bucket))
        .route("/api/store/buckets/:bucket/policy", put(put_policy))
        .route("/api/store/buckets/:bucket/policy", get(get_policy))
        .route("/api/store/:bucket/objects", get(list_objects))
        .route("/api/store/:bucket/objects/*key", put(put_object))
        .route("/api/store/:bucket/objects/*key", get(get_object))
        .route("/api/store/:bucket/objects/*key", delete(delete_object))
        .route("/api/store/:bucket/objects/*key", head(head_object))
        .route("/api/store/:bucket/multipart", post(multipart_action))
        .route("/api/store/:bucket/multipart/:upload_id", put(upload_part))
        .route("/api/store/:bucket/multipart/:upload_id", post(complete_or_abort))
        .route("/api/store/:bucket/multipart/:upload_id", delete(abort_upload))
        .with_state(state)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok", "service": "cave-store"}))
}

async fn list_buckets(State(store): State<StoreState>) -> impl IntoResponse {
    let buckets = store.list_buckets().await;
    let list: Vec<serde_json::Value> = buckets
        .into_iter()
        .map(|b| serde_json::json!({"name": b.name, "region": b.region, "created_at": b.created_at}))
        .collect();
    Json(serde_json::json!({"buckets": list}))
}

#[derive(Deserialize)]
struct CreateBucketQuery {
    region: Option<String>,
}

async fn create_bucket(
    State(store): State<StoreState>,
    Path(bucket): Path<String>,
    Query(q): Query<CreateBucketQuery>,
) -> impl IntoResponse {
    let region = q.region.unwrap_or_else(|| "us-east-1".to_string());
    match store.create_bucket(&bucket, &region).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn delete_bucket(
    State(store): State<StoreState>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    match store.delete_bucket(&bucket).await {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn put_policy(
    State(store): State<StoreState>,
    Path(bucket): Path<String>,
    Json(policy): Json<BucketPolicy>,
) -> impl IntoResponse {
    match store.put_bucket_policy(&bucket, policy).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn get_policy(
    State(store): State<StoreState>,
    Path(bucket): Path<String>,
) -> impl IntoResponse {
    match store.get_bucket_policy(&bucket).await {
        Ok(policy) => Json(policy).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/jovial-faraday
}

#[derive(Deserialize)]
struct ListObjectsQuery {
    prefix: Option<String>,
    max_keys: Option<usize>,
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/jovial-faraday
}

#[derive(Deserialize)]
struct PutObjectRequest {
    bucket: String,
    key: String,
    content: serde_json::Value,
    content_type: Option<String>,
    metadata: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct GetObjectQuery {
    version_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct InitiateMultipartRequest {
    bucket: String,
    key: String,
    content_type: Option<String>,
    metadata: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct UploadPartQuery {
    part_number: u32,
}

#[derive(Deserialize)]
struct UploadPartBody {
    content: serde_json::Value,
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ObjectMetadata {
    key: String,
    bucket: String,
    size: u64,
    content_type: String,
    etag: String,
    version_id: Option<Uuid>,
    is_delete_marker: bool,
    last_modified: chrono::DateTime<Utc>,
    metadata: HashMap<String, String>,
}

impl From<StorageObject> for ObjectMetadata {
    fn from(o: StorageObject) -> Self {
        Self {
            key: o.key,
            bucket: o.bucket,
            size: o.size,
            content_type: o.content_type,
            etag: o.etag,
            version_id: o.version_id,
            is_delete_marker: o.is_delete_marker,
            last_modified: o.last_modified,
            metadata: o.metadata,
        }
    }
}

// ── Error helpers ─────────────────────────────────────────────────────────────

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<serde_json::Value>)>;

fn err_not_found(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": msg })))
}

fn err_conflict(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::CONFLICT, Json(serde_json::json!({ "error": msg })))
}

fn err_bad_request(msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": msg })))
}

fn store_err(e: String) -> (StatusCode, Json<serde_json::Value>) {
    if e.contains("not found") {
        err_not_found(&e)
    } else if e.contains("already exists") || e.contains("not empty") {
        err_conflict(&e)
    } else {
        err_bad_request(&e)
    }
}

// ── Bucket handlers ───────────────────────────────────────────────────────────

async fn list_buckets(State(state): State<Arc<StoreState>>) -> Json<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    let buckets = store.list_buckets();
    Json(serde_json::json!({ "buckets": buckets, "count": buckets.len() }))
}

async fn create_bucket(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<CreateBucketRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.create_bucket(req.name, req.region, req.tags) {
        Ok(bucket) => Ok(Json(serde_json::json!({ "bucket": bucket }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn get_bucket(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    match store.get_bucket(&bucket) {
        Some(b) => Ok(Json(serde_json::json!({ "bucket": b }))),
        None => Err(err_not_found(&format!("bucket '{bucket}' not found"))),
    }
}

async fn delete_bucket(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.delete_bucket(&bucket) {
        Ok(()) => Ok(Json(serde_json::json!({ "deleted": true, "bucket": bucket }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn set_versioning(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
    Json(req): Json<SetVersioningRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.set_versioning(&bucket, req.enabled) {
        Ok(()) => Ok(Json(serde_json::json!({ "bucket": bucket, "versioning": req.enabled }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn get_lifecycle(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    match store.get_bucket(&bucket) {
        Some(b) => Ok(Json(serde_json::json!({ "rules": b.lifecycle_rules }))),
        None => Err(err_not_found(&format!("bucket '{bucket}' not found"))),
    }
}

async fn put_lifecycle(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
    Json(req): Json<PutLifecycleRequest>,
) -> ApiResult<serde_json::Value> {
    let rules: Vec<LifecycleRule> = req
        .rules
        .into_iter()
        .map(|r| LifecycleRule {
            id: Uuid::new_v4(),
            prefix: r.prefix,
            expiration_days: r.expiration_days,
            transition_storage_class: r.transition_storage_class,
            enabled: r.enabled,
            created_at: Utc::now(),
        })
        .collect();
    let mut store = state.inner.lock().unwrap();
    match store.set_lifecycle_rules(&bucket, rules) {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true, "bucket": bucket }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn get_policy(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    match store.get_bucket(&bucket) {
        Some(b) => Ok(Json(serde_json::json!({ "policy": b.access_policy }))),
        None => Err(err_not_found(&format!("bucket '{bucket}' not found"))),
    }
}

async fn put_policy(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
    Json(policy): Json<AccessPolicy>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.set_access_policy(&bucket, policy) {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true, "bucket": bucket }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn get_replication(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    match store.get_bucket(&bucket) {
        Some(b) => Ok(Json(serde_json::json!({ "rules": b.replication_rules }))),
        None => Err(err_not_found(&format!("bucket '{bucket}' not found"))),
    }
}

async fn put_replication(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
    Json(req): Json<PutReplicationRequest>,
) -> ApiResult<serde_json::Value> {
    let rules: Vec<ReplicationRule> = req
        .rules
        .into_iter()
        .map(|r| ReplicationRule {
            id: Uuid::new_v4(),
            destination_bucket: r.destination_bucket,
            prefix: r.prefix,
            enabled: r.enabled,
            created_at: Utc::now(),
        })
        .collect();
    let mut store = state.inner.lock().unwrap();
    match store.set_replication_rules(&bucket, rules) {
        Ok(()) => Ok(Json(serde_json::json!({ "ok": true, "bucket": bucket }))),
        Err(e) => Err(store_err(e)),
    }
}

// ── Object handlers ───────────────────────────────────────────────────────────

async fn list_objects(
    Path(bucket): Path<String>,
    Query(q): Query<ListObjectsQuery>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    match store.list_objects(&bucket, q.prefix.as_deref(), q.max_keys) {
        Ok(objects) => {
            let meta: Vec<ObjectMetadata> = objects.into_iter().map(Into::into).collect();
            Ok(Json(serde_json::json!({ "objects": meta, "count": meta.len() })))
        }
        Err(e) => Err(store_err(e)),
<<<<<<< HEAD
=======
    continuation_token: Option<String>,
    delimiter: Option<String>,
}

async fn list_objects(
    State(store): State<StoreState>,
    Path(bucket): Path<String>,
    Query(q): Query<ListObjectsQuery>,
) -> impl IntoResponse {
    match store
        .list_objects_v2(
            &bucket,
            q.prefix.as_deref(),
            q.delimiter.as_deref(),
            q.max_keys,
            q.continuation_token.as_deref(),
        )
        .await
    {
        Ok(result) => {
            let objects: Vec<serde_json::Value> = result
                .objects
                .iter()
                .map(|o| {
                    serde_json::json!({
                        "key": o.key,
                        "size": o.size,
                        "etag": o.etag,
                        "last_modified": o.last_modified,
                    })
                })
                .collect();
            Json(serde_json::json!({
                "objects": objects,
                "common_prefixes": result.common_prefixes,
                "is_truncated": result.is_truncated,
                "next_continuation_token": result.next_continuation_token,
            }))
            .into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/jovial-faraday
    }
}

async fn put_object(
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/jovial-faraday
    State(state): State<Arc<StoreState>>,
    Json(req): Json<PutObjectRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.put_object(&req.bucket, req.key, req.content, req.content_type, req.metadata) {
        Ok(obj) => {
            let meta: ObjectMetadata = obj.into();
            Ok(Json(serde_json::json!({ "object": meta })))
        }
        Err(e) => Err(store_err(e)),
<<<<<<< HEAD
=======
    State(store): State<StoreState>,
    Path((bucket, key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    match store
        .put_object(&bucket, &key, body.to_vec(), &content_type, HashMap::new(), None)
        .await
    {
        Ok(info) => (StatusCode::OK, Json(serde_json::json!({
            "etag": info.etag,
            "version_id": info.version_id,
        })))
        .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/jovial-faraday
    }
}

async fn get_object(
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/jovial-faraday
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<GetObjectQuery>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    match store.get_object(&bucket, &key, q.version_id) {
        Some(obj) => Ok(Json(serde_json::json!({
            "key": obj.key,
            "bucket": obj.bucket,
            "size": obj.size,
            "content_type": obj.content_type,
            "etag": obj.etag,
            "version_id": obj.version_id,
            "last_modified": obj.last_modified,
            "metadata": obj.metadata,
            "content": obj.content,
        }))),
        None => Err(err_not_found(&format!("object '{key}' not found in '{bucket}'"))),
<<<<<<< HEAD
=======
    State(store): State<StoreState>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.get_object(&bucket, &key, None).await {
        Ok((version, data)) => (
            StatusCode::OK,
            [
                ("content-type", version.content_type.clone()),
                ("etag", version.etag.clone()),
            ],
            data,
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/jovial-faraday
    }
}

async fn delete_object(
<<<<<<< HEAD
<<<<<<< HEAD
=======
>>>>>>> claude/jovial-faraday
    Path((bucket, key)): Path<(String, String)>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.delete_object(&bucket, &key) {
        Ok(version_id) => Ok(Json(serde_json::json!({
            "deleted": true,
            "bucket": bucket,
            "key": key,
            "delete_marker_version": version_id,
        }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn list_versions(
    Path((bucket, key)): Path<(String, String)>,
    State(state): State<Arc<StoreState>>,
) -> Json<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    let versions: Vec<ObjectMetadata> = store
        .list_object_versions(&bucket, &key)
        .into_iter()
        .map(Into::into)
        .collect();
    Json(serde_json::json!({ "versions": versions, "count": versions.len() }))
}

// ── Multipart handlers ────────────────────────────────────────────────────────

async fn initiate_multipart(
    State(state): State<Arc<StoreState>>,
    Json(req): Json<InitiateMultipartRequest>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.initiate_multipart(&req.bucket, req.key, req.content_type, req.metadata) {
        Ok(upload_id) => Ok(Json(serde_json::json!({
            "upload_id": upload_id,
            "bucket": req.bucket,
        }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn upload_part(
    Path(upload_id): Path<Uuid>,
    Query(q): Query<UploadPartQuery>,
    State(state): State<Arc<StoreState>>,
    Json(body): Json<UploadPartBody>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.upload_part(upload_id, q.part_number, body.content) {
        Ok(etag) => Ok(Json(serde_json::json!({
            "upload_id": upload_id,
            "part_number": q.part_number,
            "etag": etag,
        }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn complete_multipart(
    Path(upload_id): Path<Uuid>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.complete_multipart(upload_id) {
        Ok(obj) => {
            let meta: ObjectMetadata = obj.into();
            Ok(Json(serde_json::json!({ "object": meta })))
        }
        Err(e) => Err(store_err(e)),
    }
}

async fn abort_multipart(
    Path(upload_id): Path<Uuid>,
    State(state): State<Arc<StoreState>>,
) -> ApiResult<serde_json::Value> {
    let mut store = state.inner.lock().unwrap();
    match store.abort_multipart(upload_id) {
        Ok(()) => Ok(Json(serde_json::json!({ "aborted": true, "upload_id": upload_id }))),
        Err(e) => Err(store_err(e)),
    }
}

async fn list_multipart(
    Path(bucket): Path<String>,
    State(state): State<Arc<StoreState>>,
) -> Json<serde_json::Value> {
    let store = state.inner.lock().unwrap();
    let uploads: Vec<_> = store
        .list_multipart_uploads(&bucket)
        .into_iter()
        .map(|u| {
            serde_json::json!({
                "upload_id": u.upload_id,
                "key": u.key,
                "initiated_at": u.initiated_at,
                "parts": u.parts.len(),
            })
        })
        .collect();
    Json(serde_json::json!({ "uploads": uploads, "count": uploads.len() }))
}
<<<<<<< HEAD
=======
    State(store): State<StoreState>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.delete_object(&bucket, &key, None).await {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

async fn head_object(
    State(store): State<StoreState>,
    Path((bucket, key)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.head_object(&bucket, &key).await {
        Ok(info) => (
            StatusCode::OK,
            [
                ("content-type", info.content_type),
                ("etag", info.etag),
                ("content-length", info.size.to_string()),
            ],
        )
            .into_response(),
        Err(e) => (StatusCode::NOT_FOUND, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct MultipartActionQuery {
    action: Option<String>,
    key: Option<String>,
}

async fn multipart_action(
    State(store): State<StoreState>,
    Path(bucket): Path<String>,
    Query(q): Query<MultipartActionQuery>,
) -> impl IntoResponse {
    match q.action.as_deref() {
        Some("create") => {
            let key = q.key.unwrap_or_default();
            match store.create_multipart_upload(&bucket, &key, HashMap::new()).await {
                Ok(upload_id) => Json(serde_json::json!({"upload_id": upload_id})).into_response(),
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
            }
        }
        _ => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "unknown action"}))).into_response(),
    }
}

#[derive(Deserialize)]
struct UploadPartQuery {
    part: Option<u32>,
}

async fn upload_part(
    State(store): State<StoreState>,
    Path((_bucket, upload_id)): Path<(String, String)>,
    Query(q): Query<UploadPartQuery>,
    body: Bytes,
) -> impl IntoResponse {
    let part_number = q.part.unwrap_or(1);
    match store.upload_part(&upload_id, part_number, body.to_vec()).await {
        Ok(etag) => Json(serde_json::json!({"etag": etag, "part": part_number})).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

#[derive(Deserialize)]
struct CompleteActionQuery {
    action: Option<String>,
}

#[derive(Deserialize)]
struct CompleteParts {
    parts: Vec<(u32, String)>,
}

async fn complete_or_abort(
    State(store): State<StoreState>,
    Path((_bucket, upload_id)): Path<(String, String)>,
    Query(q): Query<CompleteActionQuery>,
    Json(body): Json<CompleteParts>,
) -> impl IntoResponse {
    match q.action.as_deref() {
        Some("complete") | None => {
            let parts: Vec<(u32, String)> = body.parts;
            match store.complete_multipart_upload(&upload_id, parts).await {
                Ok(info) => Json(serde_json::json!({"etag": info.etag})).into_response(),
                Err(e) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
            }
        }
        _ => (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "unknown action"}))).into_response(),
    }
}

async fn abort_upload(
    State(store): State<StoreState>,
    Path((_bucket, upload_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match store.abort_multipart_upload(&upload_id).await {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}
>>>>>>> claude/dazzling-tesla
=======
>>>>>>> claude/jovial-faraday
