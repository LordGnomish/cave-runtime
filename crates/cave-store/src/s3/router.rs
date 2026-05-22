// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! S3-compatible HTTP router — full AWS S3 API surface via axum.

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, head, post, put},
    Router,
};
use bytes::Bytes;
use http_body_util::BodyExt;
use tracing::warn;

use crate::error::StoreError;
use super::{
    store::S3Store,
    types::*,
};

// ─── State ────────────────────────────────────────────────────────────────────

pub fn s3_router(store: Arc<S3Store>) -> Router {
    Router::new()
        // Service-level
        .route("/", get(list_buckets))
        // Bucket-level (sub-resources before plain bucket)
        .route("/{bucket}", put(put_bucket_dispatch))
        .route("/{bucket}", delete(delete_bucket))
        .route("/{bucket}", head(head_bucket))
        .route("/{bucket}", get(get_bucket_dispatch))
        .route("/{bucket}", post(post_bucket_dispatch))
        // Object-level
        .route("/{bucket}/{*key}", put(put_object_dispatch))
        .route("/{bucket}/{*key}", get(get_object))
        .route("/{bucket}/{*key}", head(head_object))
        .route("/{bucket}/{*key}", delete(delete_object))
        .route("/{bucket}/{*key}", post(post_object_dispatch))
        .with_state(store)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

const XML_CT: &str = "application/xml";

fn xml_resp(status: StatusCode, body: String) -> Response {
    (status, [(axum::http::header::CONTENT_TYPE, XML_CT)], body).into_response()
}

fn err_resp(e: StoreError, resource: &str) -> Response {
    let (status, code, msg) = match &e {
        StoreError::BucketNotFound(_) => (StatusCode::NOT_FOUND, "NoSuchBucket", "The specified bucket does not exist"),
        StoreError::BucketExists(_) => (StatusCode::CONFLICT, "BucketAlreadyExists", "The requested bucket name is not available"),
        StoreError::ObjectNotFound(_, _) => (StatusCode::NOT_FOUND, "NoSuchKey", "The specified key does not exist"),
        StoreError::InvalidRequest(msg) => (StatusCode::BAD_REQUEST, "InvalidArgument", msg.as_str()),
        StoreError::UploadNotFound(_) => (StatusCode::NOT_FOUND, "NoSuchUpload", "The specified upload does not exist"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "InternalError", "An internal error occurred"),
    };
    xml_resp(status, error_xml(code, msg, resource))
}

async fn body_bytes(req: Request) -> Result<Bytes, Response> {
    let body = req.into_body();
    body.collect()
        .await
        .map(|c| c.to_bytes())
        .map_err(|e| {
            (StatusCode::BAD_REQUEST, format!("failed to read body: {e}")).into_response()
        })
}

fn parse_sse(headers: &HeaderMap) -> Option<SseConfig> {
    let alg = headers.get("x-amz-server-side-encryption")?.to_str().ok()?;
    match alg {
        "AES256" => Some(SseConfig { algorithm: SseAlgorithm::AwsS3, customer_key: None }),
        "aws:kms" => Some(SseConfig { algorithm: SseAlgorithm::AwsKms, customer_key: None }),
        _ => None,
    }
}

fn parse_sse_c(headers: &HeaderMap) -> Option<SseConfig> {
    let _alg = headers.get("x-amz-server-side-encryption-customer-algorithm")?;
    let key_b64 = headers.get("x-amz-server-side-encryption-customer-key")?.to_str().ok()?;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let key = B64.decode(key_b64).ok()?;
    Some(SseConfig { algorithm: SseAlgorithm::Customer, customer_key: Some(key) })
}

fn parse_range(headers: &HeaderMap) -> Option<(u64, u64)> {
    let range_header = headers.get("range")?.to_str().ok()?;
    // Expect: "bytes=start-end"
    let stripped = range_header.strip_prefix("bytes=")?;
    let mut parts = stripped.splitn(2, '-');
    let start: u64 = parts.next()?.parse().ok()?;
    let end: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(u64::MAX);
    Some((start, end))
}

fn user_metadata(headers: &HeaderMap) -> HashMap<String, String> {
    headers.iter()
        .filter_map(|(k, v)| {
            let name = k.as_str();
            if let Some(stripped) = name.strip_prefix("x-amz-meta-") {
                v.to_str().ok().map(|val| (stripped.to_string(), val.to_string()))
            } else {
                None
            }
        })
        .collect()
}

// ─── Service-level ────────────────────────────────────────────────────────────

async fn list_buckets(State(store): State<Arc<S3Store>>) -> Response {
    let buckets = store.list_buckets();
    xml_resp(StatusCode::OK, list_buckets_xml(&buckets))
}

// ─── Bucket-level ─────────────────────────────────────────────────────────────

async fn put_bucket_dispatch(
    State(store): State<Arc<S3Store>>,
    Path(bucket): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    if params.contains_key("versioning") {
        return put_bucket_versioning(store, bucket, req).await;
    }
    if params.contains_key("lifecycle") {
        return put_bucket_lifecycle(store, bucket, req).await;
    }
    if params.contains_key("policy") {
        return put_bucket_policy(store, bucket, req).await;
    }
    if params.contains_key("notification") {
        return put_bucket_notification(store, bucket, req).await;
    }
    if params.contains_key("acl") {
        return put_bucket_acl(store, bucket, req).await;
    }
    // Default: create bucket
    let region = headers
        .get("x-amz-bucket-region")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("us-east-1")
        .to_string();
    match store.create_bucket(bucket.clone(), region) {
        Ok(()) => (StatusCode::OK, "").into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn put_bucket_versioning(store: Arc<S3Store>, bucket: String, req: Request) -> Response {
    let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
    let body = String::from_utf8_lossy(&bytes);
    let status = if body.contains("Enabled") {
        VersioningStatus::Enabled
    } else if body.contains("Suspended") {
        VersioningStatus::Suspended
    } else {
        VersioningStatus::Off
    };
    match store.put_bucket_versioning(&bucket, BucketVersioning { status }) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn put_bucket_lifecycle(store: Arc<S3Store>, bucket: String, req: Request) -> Response {
    let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
    let body = String::from_utf8_lossy(&bytes);
    // Minimal XML parse — extract rules
    let rules = parse_lifecycle_xml(&body);
    match store.put_bucket_lifecycle(&bucket, rules) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

fn parse_lifecycle_xml(xml: &str) -> Vec<LifecycleRule> {
    // Lightweight extraction without full XML parse
    let mut rules = Vec::new();
    for rule_chunk in xml.split("<Rule>").skip(1) {
        let id = extract_xml_text(rule_chunk, "ID").unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let prefix = extract_xml_text(rule_chunk, "Prefix").unwrap_or_default();
        let enabled = extract_xml_text(rule_chunk, "Status").map(|s| s == "Enabled").unwrap_or(true);
        let expiration_days = extract_xml_text(rule_chunk, "Days").and_then(|d| d.parse().ok());
        rules.push(LifecycleRule { id, prefix, expiration_days, noncurrent_version_expiration_days: None, enabled });
    }
    rules
}

fn extract_xml_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    Some(xml[start..start + end].to_string())
}

async fn put_bucket_policy(store: Arc<S3Store>, bucket: String, req: Request) -> Response {
    let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
    match serde_json::from_slice::<BucketPolicy>(&bytes) {
        Ok(policy) => match store.put_bucket_policy(&bucket, policy) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => err_resp(e, &format!("/{bucket}")),
        },
        Err(_) => (StatusCode::BAD_REQUEST, "invalid policy JSON").into_response(),
    }
}

async fn put_bucket_notification(store: Arc<S3Store>, bucket: String, req: Request) -> Response {
    let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
    let config = serde_json::from_slice::<NotificationConfig>(&bytes).unwrap_or_default();
    match store.put_bucket_notification(&bucket, config) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn put_bucket_acl(store: Arc<S3Store>, bucket: String, _req: Request) -> Response {
    // Accept pre-canned ACLs; for simplicity we store default ACL
    match store.put_bucket_acl(&bucket, BucketAcl::default()) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn delete_bucket(
    State(store): State<Arc<S3Store>>,
    Path(bucket): Path<String>,
) -> Response {
    match store.delete_bucket(&bucket) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn head_bucket(
    State(store): State<Arc<S3Store>>,
    Path(bucket): Path<String>,
) -> Response {
    match store.head_bucket(&bucket) {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_bucket_dispatch(
    State(store): State<Arc<S3Store>>,
    Path(bucket): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if params.get("list-type").map(|s| s == "2").unwrap_or(false) {
        return list_objects_v2(store, bucket, params).await;
    }
    if params.contains_key("uploads") {
        return list_multipart_uploads(store, bucket).await;
    }
    if params.contains_key("versioning") {
        return get_bucket_versioning(store, bucket).await;
    }
    if params.contains_key("lifecycle") {
        return get_bucket_lifecycle(store, bucket).await;
    }
    if params.contains_key("policy") {
        return get_bucket_policy(store, bucket).await;
    }
    if params.contains_key("notification") {
        return get_bucket_notification(store, bucket).await;
    }
    if params.contains_key("acl") {
        return get_bucket_acl(store, bucket).await;
    }
    // Default: ListObjectsV2 without list-type=2 (ListObjects v1 compatible)
    list_objects_v2(store, bucket, params).await
}

async fn list_objects_v2(
    store: Arc<S3Store>,
    bucket: String,
    params: HashMap<String, String>,
) -> Response {
    let prefix = params.get("prefix").cloned().unwrap_or_default();
    let delimiter = params.get("delimiter").cloned().unwrap_or_default();
    let continuation_token = params.get("continuation-token").cloned();
    let max_keys: u32 = params.get("max-keys").and_then(|v| v.parse().ok()).unwrap_or(1000);

    match store.list_objects_v2(&bucket, &prefix, &delimiter, continuation_token.as_deref(), max_keys) {
        Ok(result) => xml_resp(StatusCode::OK, list_objects_v2_xml(&result)),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn list_multipart_uploads(store: Arc<S3Store>, bucket: String) -> Response {
    match store.list_multipart_uploads(&bucket) {
        Ok(uploads) => xml_resp(StatusCode::OK, list_multipart_uploads_xml(&bucket, &uploads)),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn get_bucket_versioning(store: Arc<S3Store>, bucket: String) -> Response {
    match store.get_bucket_versioning(&bucket) {
        Ok(v) => xml_resp(StatusCode::OK, versioning_xml(&v)),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn get_bucket_lifecycle(store: Arc<S3Store>, bucket: String) -> Response {
    match store.get_bucket_lifecycle(&bucket) {
        Ok(rules) => {
            let body = format!(
                r#"<?xml version="1.0"?><LifecycleConfiguration>{}</LifecycleConfiguration>"#,
                rules.iter().map(|r| format!(
                    "<Rule><ID>{}</ID><Prefix>{}</Prefix><Status>{}</Status></Rule>",
                    r.id, r.prefix, if r.enabled { "Enabled" } else { "Disabled" }
                )).collect::<String>()
            );
            xml_resp(StatusCode::OK, body)
        }
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn get_bucket_policy(store: Arc<S3Store>, bucket: String) -> Response {
    match store.get_bucket_policy(&bucket) {
        Ok(policy) => {
            let json = serde_json::to_string(&policy).unwrap_or_default();
            (StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], json).into_response()
        }
        Err(_) => err_resp(StoreError::InvalidRequest("no policy".into()), &format!("/{bucket}")),
    }
}

async fn get_bucket_notification(store: Arc<S3Store>, bucket: String) -> Response {
    match store.get_bucket_notification(&bucket) {
        Ok(cfg) => {
            let json = serde_json::to_string(&cfg).unwrap_or_default();
            (StatusCode::OK, json).into_response()
        }
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn get_bucket_acl(store: Arc<S3Store>, bucket: String) -> Response {
    match store.get_bucket_acl(&bucket) {
        Ok(acl) => xml_resp(StatusCode::OK, acl_xml(&acl)),
        Err(e) => err_resp(e, &format!("/{bucket}")),
    }
}

async fn post_bucket_dispatch(
    State(_store): State<Arc<S3Store>>,
    Path(bucket): Path<String>,
    Query(_params): Query<HashMap<String, String>>,
) -> Response {
    warn!("unhandled POST to bucket /{bucket}");
    StatusCode::NOT_IMPLEMENTED.into_response()
}

// ─── Object-level ─────────────────────────────────────────────────────────────

async fn put_object_dispatch(
    State(store): State<Arc<S3Store>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    // CreateMultipartUpload
    if params.contains_key("uploads") {
        let metadata = user_metadata(&headers);
        let ct = headers.get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        return match store.create_multipart_upload(&bucket, &key, metadata, ct) {
            Ok(upload_id) => xml_resp(StatusCode::OK, initiate_multipart_xml(&bucket, &key, &upload_id)),
            Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
        };
    }

    // UploadPart
    if let (Some(part_str), Some(upload_id)) = (params.get("partNumber"), params.get("uploadId")) {
        let part_number: u32 = match part_str.parse() {
            Ok(n) => n,
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid partNumber").into_response(),
        };
        let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
        return match store.upload_part(&bucket, &key, upload_id, part_number, bytes) {
            Ok(etag) => {
                let mut resp = StatusCode::OK.into_response();
                resp.headers_mut().insert(
                    "ETag",
                    format!("\"{etag}\"").parse().unwrap(),
                );
                resp
            }
            Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
        };
    }

    // CopyObject (x-amz-copy-source)
    if let Some(copy_src) = headers.get("x-amz-copy-source").and_then(|v| v.to_str().ok()) {
        let (src_bucket, src_key) = parse_copy_source(copy_src);
        return match store.copy_object(&src_bucket, &src_key, &bucket, &key) {
            Ok(meta) => {
                let body = format!(
                    r#"<?xml version="1.0"?><CopyObjectResult><LastModified>{}</LastModified><ETag>&quot;{}&quot;</ETag></CopyObjectResult>"#,
                    fmt_time(&meta.last_modified), meta.etag
                );
                xml_resp(StatusCode::OK, body)
            }
            Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
        };
    }

    // PutObject
    let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
    let metadata = user_metadata(&headers);
    let ct = headers.get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let sse = parse_sse_c(&headers).or_else(|| parse_sse(&headers));

    match store.put_object(&bucket, &key, bytes, metadata, ct, sse) {
        Ok(meta) => {
            let mut resp = StatusCode::OK.into_response();
            resp.headers_mut().insert("ETag", format!("\"{}\""  , meta.etag).parse().unwrap());
            resp
        }
        Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
    }
}

async fn get_object(
    State(store): State<Arc<S3Store>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let version_id = params.get("versionId").map(|s| s.as_str());
    let range = parse_range(&headers);
    let customer_key = parse_sse_c(&headers).and_then(|s| s.customer_key);

    match store.get_object(&bucket, &key, version_id, range, customer_key.as_deref()) {
        Ok((meta, data)) => {
            let status = if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
            let mut resp = (status, Body::from(data)).into_response();
            let h = resp.headers_mut();
            h.insert(axum::http::header::CONTENT_TYPE, meta.content_type.parse().unwrap_or(
                "application/octet-stream".parse().unwrap(),
            ));
            h.insert("ETag", format!("\"{}\"", meta.etag).parse().unwrap());
            h.insert("Last-Modified", fmt_time(&meta.last_modified).parse().unwrap());
            h.insert("Content-Length", meta.size.to_string().parse().unwrap());
            if let Some(vid) = meta.version_id {
                h.insert("x-amz-version-id", vid.parse().unwrap());
            }
            resp
        }
        Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
    }
}

async fn head_object(
    State(store): State<Arc<S3Store>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let version_id = params.get("versionId").map(|s| s.as_str());
    match store.head_object(&bucket, &key, version_id) {
        Ok(meta) => {
            let mut resp = StatusCode::OK.into_response();
            let h = resp.headers_mut();
            h.insert(axum::http::header::CONTENT_TYPE, meta.content_type.parse().unwrap_or(
                "application/octet-stream".parse().unwrap(),
            ));
            h.insert("ETag", format!("\"{}\"", meta.etag).parse().unwrap());
            h.insert("Content-Length", meta.size.to_string().parse().unwrap());
            resp
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_object(
    State(store): State<Arc<S3Store>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let version_id = params.get("versionId").map(|s| s.as_str());
    match store.delete_object(&bucket, &key, version_id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
    }
}

async fn post_object_dispatch(
    State(store): State<Arc<S3Store>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
) -> Response {
    let upload_id = match params.get("uploadId") {
        Some(id) => id.clone(),
        None => return StatusCode::BAD_REQUEST.into_response(),
    };

    let Ok(bytes) = body_bytes(req).await else { return StatusCode::BAD_REQUEST.into_response() };
    let body = String::from_utf8_lossy(&bytes);

    // Check if this is AbortMultipartUpload (DELETE method would normally be used,
    // but some clients POST abort; check for empty body or x-id=AbortMultipartUpload)
    if params.get("x-id").map(|s| s == "AbortMultipartUpload").unwrap_or(false) || body.is_empty() {
        return match store.abort_multipart_upload(&bucket, &key, &upload_id) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
        };
    }

    // CompleteMultipartUpload
    let parts = parse_complete_multipart_xml(&body);
    match store.complete_multipart_upload(&bucket, &key, &upload_id, parts) {
        Ok(meta) => {
            let location = format!("/{bucket}/{key}");
            xml_resp(StatusCode::OK, complete_multipart_xml(&bucket, &key, &meta.etag, &location))
        }
        Err(e) => err_resp(e, &format!("/{bucket}/{key}")),
    }
}

fn parse_complete_multipart_xml(xml: &str) -> Vec<(u32, String)> {
    let mut parts = Vec::new();
    for chunk in xml.split("<Part>").skip(1) {
        let part_num: u32 = extract_xml_text(chunk, "PartNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let etag = extract_xml_text(chunk, "ETag")
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        if part_num > 0 {
            parts.push((part_num, etag));
        }
    }
    parts
}

fn parse_copy_source(src: &str) -> (String, String) {
    let src = src.trim_start_matches('/');
    if let Some(pos) = src.find('/') {
        (src[..pos].to_string(), src[pos + 1..].to_string())
    } else {
        (src.to_string(), String::new())
    }
}
