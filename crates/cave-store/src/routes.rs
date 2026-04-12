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
}

#[derive(Deserialize)]
struct ListObjectsQuery {
    prefix: Option<String>,
    max_keys: Option<usize>,
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
    }
}

async fn put_object(
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
    }
}

async fn get_object(
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
    }
}

async fn delete_object(
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
