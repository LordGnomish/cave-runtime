// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
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
            "/api/v1/store/buckets/{bucket}",
            get(get_bucket).delete(delete_bucket),
        )
        // Bucket configuration
        .route(
            "/api/v1/store/buckets/{bucket}/versioning",
            put(set_versioning),
        )
        .route(
            "/api/v1/store/buckets/{bucket}/lifecycle",
            get(get_lifecycle).put(put_lifecycle),
        )
        .route(
            "/api/v1/store/buckets/{bucket}/policy",
            get(get_policy).put(put_policy),
        )
        .route(
            "/api/v1/store/buckets/{bucket}/replication",
            get(get_replication).put(put_replication),
        )
        // Object CRUD
        .route(
            "/api/v1/store/buckets/{bucket}/objects",
            get(list_objects),
        )
        .route("/api/v1/store/objects", post(put_object))
        .route(
            "/api/v1/store/objects/{bucket}/{key}",
            get(get_object).delete(delete_object),
        )
        .route(
            "/api/v1/store/objects/{bucket}/{key}/versions",
            get(list_versions),
        )
        // Multipart upload
        .route("/api/v1/store/multipart/initiate", post(initiate_multipart))
        .route(
            "/api/v1/store/multipart/{upload_id}/parts",
            put(upload_part),
        )
        .route(
            "/api/v1/store/multipart/{upload_id}/complete",
            post(complete_multipart),
        )
        .route(
            "/api/v1/store/multipart/{upload_id}",
            delete(abort_multipart),
        )
        .route(
            "/api/v1/store/buckets/{bucket}/multipart",
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
}

#[derive(Deserialize)]
struct ListObjectsQuery {
    prefix: Option<String>,
    max_keys: Option<usize>,
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
    }
}

async fn put_object(
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
    }
}

async fn get_object(
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
    }
}

async fn delete_object(
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
