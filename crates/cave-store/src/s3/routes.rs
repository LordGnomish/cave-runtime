//! S3/MinIO HTTP routes.
//!
//! Implements the S3 REST API:
//!   Buckets: PUT /{bucket}, DELETE /{bucket}, GET /, GET /{bucket}
//!   Objects: PUT /{bucket}/{key}, GET /{bucket}/{key}, DELETE /{bucket}/{key}, HEAD /{bucket}/{key}
//!   Multipart: POST /{bucket}/{key}?uploads, PUT /{bucket}/{key}?uploadId=&partNumber=,
//!              POST /{bucket}/{key}?uploadId=, DELETE /{bucket}/{key}?uploadId=
//!   Bucket sub-resources: ?versioning, ?policy, ?lifecycle, ?notification, ?tagging,
//!                         ?acl, ?encryption, ?location

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, head, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::s3::presigned;
use crate::s3::store::{DeleteObjectEntry, ObjectStore};
use crate::s3::types::{
    BucketEncryption, BucketAcl, LifecycleRule, NotificationConfiguration,
    SseAlgorithm, StorageClass, VersioningState,
};
use crate::s3::xml;
use crate::StoreState;

const XML_CONTENT_TYPE: &str = "application/xml";

fn xml_response(status: StatusCode, body: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, XML_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

fn s3_error(status: StatusCode, code: &str, message: &str, resource: &str) -> Response {
    xml_response(status, xml::error_response(code, message, resource))
}

fn store_error_response(e: crate::error::StoreError, resource: &str) -> Response {
    let status = StatusCode::from_u16(e.s3_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    s3_error(status, e.s3_code(), &e.to_string(), resource)
}

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct BucketQuery {
    versioning: Option<String>,
    policy: Option<String>,
    lifecycle: Option<String>,
    notification: Option<String>,
    tagging: Option<String>,
    acl: Option<String>,
    encryption: Option<String>,
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ObjectQuery {
    #[serde(rename = "uploadId")]
    upload_id: Option<String>,
    #[serde(rename = "partNumber")]
    part_number: Option<u32>,
    uploads: Option<String>,
    #[serde(rename = "versionId")]
    version_id: Option<String>,
    tagging: Option<String>,
    #[serde(rename = "X-Cave-Algorithm")]
    cave_algorithm: Option<String>,
    #[serde(rename = "X-Cave-Credential")]
    cave_credential: Option<String>,
    #[serde(rename = "X-Cave-Expires")]
    cave_expires: Option<i64>,
    #[serde(rename = "X-Cave-Signature")]
    cave_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListObjectsQuery {
    prefix: Option<String>,
    delimiter: Option<String>,
    #[serde(rename = "max-keys")]
    max_keys: Option<u32>,
    #[serde(rename = "continuation-token")]
    continuation_token: Option<String>,
    #[serde(rename = "list-type")]
    list_type: Option<u8>,
    versioning: Option<String>,
}

// ── Bucket routes ─────────────────────────────────────────────────────────────

/// GET / — list all buckets
async fn list_buckets(State(state): State<Arc<StoreState>>) -> Response {
    let buckets = state.s3.list_buckets().await;
    let items: Vec<xml::BucketListItem> = buckets
        .iter()
        .map(|b| xml::BucketListItem {
            name: b.name.clone(),
            creation_date: b.created_at,
        })
        .collect();
    xml_response(StatusCode::OK, xml::list_buckets("cave", &items))
}

/// PUT /{bucket} — create bucket
async fn create_bucket(
    State(state): State<Arc<StoreState>>,
    Path(bucket): Path<String>,
    headers: HeaderMap,
    Query(q): Query<BucketQuery>,
    body: Bytes,
) -> Response {
    // Sub-resource handlers
    if q.versioning.is_some() {
        return put_bucket_versioning(state, bucket, body).await;
    }
    if q.policy.is_some() {
        return put_bucket_policy(state, bucket, body).await;
    }
    if q.lifecycle.is_some() {
        return put_bucket_lifecycle(state, bucket, body).await;
    }
    if q.notification.is_some() {
        return put_bucket_notification(state, bucket, body).await;
    }
    if q.encryption.is_some() {
        return put_bucket_encryption(state, bucket, body).await;
    }
    if q.acl.is_some() {
        return put_bucket_acl(state, bucket, body).await;
    }
    if q.tagging.is_some() {
        return put_bucket_tagging(state, bucket, body).await;
    }

    let region = headers
        .get("x-amz-bucket-region")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("us-east-1")
        .to_string();

    match state.s3.create_bucket(&bucket, &region, "cave").await {
        Ok(()) => {
            let mut resp = StatusCode::OK.into_response();
            resp.headers_mut().insert(
                header::LOCATION,
                format!("/{bucket}").parse().unwrap(),
            );
            resp
        }
        Err(e) => store_error_response(e, &format!("/{bucket}")),
    }
}

/// DELETE /{bucket}
async fn delete_bucket(
    State(state): State<Arc<StoreState>>,
    Path(bucket): Path<String>,
    Query(q): Query<BucketQuery>,
) -> Response {
    if q.policy.is_some() {
        return match state.s3.delete_bucket_policy(&bucket).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => store_error_response(e, &format!("/{bucket}?policy")),
        };
    }
    match state.s3.delete_bucket(&bucket).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}")),
    }
}

/// HEAD /{bucket}
async fn head_bucket(
    State(state): State<Arc<StoreState>>,
    Path(bucket): Path<String>,
) -> Response {
    match state.s3.head_bucket(&bucket).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => StatusCode::from_u16(e.s3_status()).unwrap_or(StatusCode::NOT_FOUND).into_response(),
    }
}

/// GET /{bucket} — list objects or sub-resource
async fn get_bucket(
    State(state): State<Arc<StoreState>>,
    Path(bucket): Path<String>,
    Query(q): Query<ListObjectsQuery>,
    Query(bq): Query<BucketQuery>,
) -> Response {
    if bq.versioning.is_some() {
        return get_bucket_versioning(state, bucket).await;
    }
    if bq.policy.is_some() {
        return get_bucket_policy(state, bucket).await;
    }
    if bq.lifecycle.is_some() {
        return get_bucket_lifecycle(state, bucket).await;
    }
    if bq.location.is_some() {
        let b = state.s3.get_bucket(&bucket).await;
        return match b {
            Ok(b) => xml_response(
                StatusCode::OK,
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?><LocationConstraint>{}</LocationConstraint>"#,
                    b.region
                ),
            ),
            Err(e) => store_error_response(e, &format!("/{bucket}")),
        };
    }

    // List objects
    if q.versioning.is_some() {
        // list-object-versions
        return list_object_versions(state, bucket, q).await;
    }

    let prefix = q.prefix.unwrap_or_default();
    let delimiter = q.delimiter.as_deref();
    let max_keys = q.max_keys.unwrap_or(1000).min(1000);

    match state
        .s3
        .list_objects_v2(
            &bucket,
            &prefix,
            delimiter,
            max_keys,
            q.continuation_token.as_deref(),
        )
        .await
    {
        Ok(result) => {
            let contents: Vec<xml::ObjectListItem> = result
                .contents
                .iter()
                .map(|obj| xml::ObjectListItem {
                    key: obj.key.clone(),
                    last_modified: obj.last_modified,
                    etag: obj.etag.clone(),
                    size: obj.size,
                    storage_class: obj.storage_class.clone(),
                    owner_id: "cave".to_string(),
                })
                .collect();
            let r = xml::ListObjectsV2Result {
                name: bucket,
                prefix,
                delimiter: q.delimiter,
                max_keys,
                key_count: result.key_count,
                is_truncated: result.is_truncated,
                next_continuation_token: result.next_continuation_token,
                contents,
                common_prefixes: result.common_prefixes,
            };
            xml_response(StatusCode::OK, xml::list_objects_v2(&r))
        }
        Err(e) => store_error_response(e, &format!("/{bucket}")),
    }
}

async fn list_object_versions(
    state: Arc<StoreState>,
    bucket: String,
    q: ListObjectsQuery,
) -> Response {
    let prefix = q.prefix.unwrap_or_default();
    match state.s3.list_object_versions(&bucket, &prefix).await {
        Ok(entries) => {
            let mut items = Vec::new();
            for (key, versions) in &entries {
                let len = versions.len();
                for (i, v) in versions.iter().enumerate() {
                    items.push(xml::VersionItem {
                        key: key.clone(),
                        version_id: v.version_id.clone().unwrap_or_else(|| "null".to_string()),
                        is_latest: i == len - 1,
                        last_modified: v.last_modified,
                        etag: v.etag.clone(),
                        size: v.size,
                        storage_class: "STANDARD".to_string(),
                        is_delete_marker: v.delete_marker,
                    });
                }
            }
            xml_response(StatusCode::OK, xml::list_object_versions(&bucket, &prefix, &items))
        }
        Err(e) => store_error_response(e, &format!("/{bucket}")),
    }
}

// Bucket sub-resource handlers

async fn put_bucket_versioning(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    let body_str = String::from_utf8_lossy(&body);
    let state_val = if body_str.contains("Enabled") {
        VersioningState::Enabled
    } else if body_str.contains("Suspended") {
        VersioningState::Suspended
    } else {
        VersioningState::Disabled
    };
    match state.s3.set_versioning(&bucket, state_val).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}?versioning")),
    }
}

async fn get_bucket_versioning(state: Arc<StoreState>, bucket: String) -> Response {
    match state.s3.get_bucket(&bucket).await {
        Ok(b) => {
            let state_str = match b.versioning {
                VersioningState::Enabled => "Enabled",
                VersioningState::Suspended => "Suspended",
                VersioningState::Disabled => "Disabled",
            };
            xml_response(StatusCode::OK, xml::get_bucket_versioning(state_str))
        }
        Err(e) => store_error_response(e, &format!("/{bucket}?versioning")),
    }
}

async fn put_bucket_policy(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    let policy_json = String::from_utf8_lossy(&body).to_string();
    match state.s3.put_bucket_policy(&bucket, &policy_json).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}?policy")),
    }
}

async fn get_bucket_policy(state: Arc<StoreState>, bucket: String) -> Response {
    match state.s3.get_bucket(&bucket).await {
        Ok(b) => match b.policy {
            Some(p) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], p)
                .into_response(),
            None => s3_error(StatusCode::NOT_FOUND, "NoSuchBucketPolicy", "no policy", &format!("/{bucket}")),
        },
        Err(e) => store_error_response(e, &format!("/{bucket}?policy")),
    }
}

async fn put_bucket_lifecycle(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    // Parse JSON lifecycle rules (simplified; real S3 uses XML)
    let rules: Vec<LifecycleRule> = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return s3_error(StatusCode::BAD_REQUEST, "MalformedXML", &e.to_string(), &format!("/{bucket}?lifecycle"));
        }
    };
    match state.s3.put_bucket_lifecycle(&bucket, rules).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}?lifecycle")),
    }
}

async fn get_bucket_lifecycle(state: Arc<StoreState>, bucket: String) -> Response {
    match state.s3.get_bucket(&bucket).await {
        Ok(b) => {
            match serde_json::to_string(&b.lifecycle_rules) {
                Ok(json) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], json).into_response(),
                Err(e) => s3_error(StatusCode::INTERNAL_SERVER_ERROR, "InternalError", &e.to_string(), &format!("/{bucket}?lifecycle")),
            }
        }
        Err(e) => store_error_response(e, &format!("/{bucket}?lifecycle")),
    }
}

async fn put_bucket_notification(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    let config: NotificationConfiguration = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            return s3_error(StatusCode::BAD_REQUEST, "MalformedXML", &e.to_string(), &format!("/{bucket}?notification"));
        }
    };
    match state.s3.put_bucket_notification(&bucket, config).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}?notification")),
    }
}

async fn put_bucket_encryption(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    // Parse JSON encryption config
    let enc: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            return s3_error(StatusCode::BAD_REQUEST, "MalformedXML", "invalid body", &format!("/{bucket}?encryption"));
        }
    };
    let algorithm = enc
        .pointer("/Rules/0/ApplyServerSideEncryptionByDefault/SSEAlgorithm")
        .and_then(|v| v.as_str())
        .unwrap_or("AES256");
    let sse_alg = if algorithm == "aws:kms" {
        SseAlgorithm::AwsKms
    } else {
        SseAlgorithm::Aes256
    };
    let kms_key = enc
        .pointer("/Rules/0/ApplyServerSideEncryptionByDefault/KMSMasterKeyID")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    match state
        .s3
        .put_bucket_encryption(&bucket, BucketEncryption { sse_algorithm: sse_alg, kms_master_key_id: kms_key })
        .await
    {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}?encryption")),
    }
}

async fn put_bucket_acl(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    // For now just return OK (ACL is tracked on the bucket)
    StatusCode::OK.into_response()
}

async fn put_bucket_tagging(state: Arc<StoreState>, bucket: String, body: Bytes) -> Response {
    let tags: HashMap<String, String> = match serde_json::from_slice(&body) {
        Ok(t) => t,
        Err(e) => {
            return s3_error(StatusCode::BAD_REQUEST, "MalformedXML", &e.to_string(), &format!("/{bucket}?tagging"));
        }
    };
    match state.s3.put_bucket_tags(&bucket, tags).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}?tagging")),
    }
}

// ── Object routes ─────────────────────────────────────────────────────────────

/// PUT /{bucket}/{key+}
async fn put_object(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Multipart part upload
    if let (Some(upload_id), Some(part_number)) = (&q.upload_id, q.part_number) {
        return upload_part(state, upload_id.clone(), part_number, body).await;
    }

    // Presigned URL verification
    if let (Some(alg), Some(cred), Some(exp), Some(sig)) = (
        &q.cave_algorithm,
        &q.cave_credential,
        q.cave_expires,
        &q.cave_signature,
    ) {
        let secret = state.s3_secret_key.as_bytes();
        if let Err(e) = presigned::verify("PUT", &bucket, &key, cred, exp, sig, secret) {
            return store_error_response(e, &format!("/{bucket}/{key}"));
        }
    }

    // Copy source
    if let Some(copy_src) = headers.get("x-amz-copy-source").and_then(|v| v.to_str().ok()) {
        let (src_bucket, src_key) = parse_copy_source(copy_src);
        let metadata_directive = headers
            .get("x-amz-metadata-directive")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("COPY");
        let new_metadata = if metadata_directive == "REPLACE" {
            Some(extract_user_metadata(&headers))
        } else {
            None
        };
        return match state
            .s3
            .copy_object(&src_bucket, &src_key, None, &bucket, &key, metadata_directive, new_metadata)
            .await
        {
            Ok(r) => {
                let body = xml::copy_object_result(&r.etag, &r.last_modified);
                let mut resp = xml_response(StatusCode::OK, body);
                if let Some(vid) = r.version_id {
                    resp.headers_mut().insert(
                        "x-amz-version-id",
                        vid.parse().unwrap(),
                    );
                }
                resp
            }
            Err(e) => store_error_response(e, &format!("/{bucket}/{key}")),
        };
    }

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let metadata = extract_user_metadata(&headers);
    let tags = extract_tags(&headers);
    let sse = headers.get("x-amz-server-side-encryption").and_then(|v| v.to_str().ok());
    let sse_c_key = headers.get("x-amz-server-side-encryption-customer-key").and_then(|v| v.to_str().ok());

    match state
        .s3
        .put_object(&bucket, &key, body.to_vec(), &content_type, metadata, tags, sse, sse_c_key, None)
        .await
    {
        Ok(r) => {
            let mut resp = StatusCode::OK.into_response();
            resp.headers_mut().insert("ETag", format!("\"{}\"", r.etag).parse().unwrap());
            if let Some(vid) = r.version_id {
                resp.headers_mut().insert("x-amz-version-id", vid.parse().unwrap());
            }
            resp
        }
        Err(e) => store_error_response(e, &format!("/{bucket}/{key}")),
    }
}

async fn upload_part(state: Arc<StoreState>, upload_id: String, part_number: u32, body: Bytes) -> Response {
    match state.s3.upload_part(&upload_id, part_number, body.to_vec()).await {
        Ok(etag) => {
            let mut resp = StatusCode::OK.into_response();
            resp.headers_mut().insert("ETag", format!("\"{}\"", etag).parse().unwrap());
            resp
        }
        Err(e) => store_error_response(e, &format!("?uploadId={upload_id}&partNumber={part_number}")),
    }
}

/// GET /{bucket}/{key+}
async fn get_object(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
    headers: HeaderMap,
) -> Response {
    if q.tagging.is_some() {
        return get_object_tagging_handler(state, bucket, key, q.version_id.as_deref()).await;
    }

    // Presigned URL check
    if let (Some(cred), Some(exp), Some(sig)) = (&q.cave_credential, q.cave_expires, &q.cave_signature) {
        let secret = state.s3_secret_key.as_bytes();
        if let Err(e) = presigned::verify("GET", &bucket, &key, cred, exp, sig, secret) {
            return store_error_response(e, &format!("/{bucket}/{key}"));
        }
    }

    // Range header
    let range = parse_range_header(headers.get(header::RANGE).and_then(|v| v.to_str().ok()));
    let sse_c_key = headers.get("x-amz-server-side-encryption-customer-key").and_then(|v| v.to_str().ok());

    match state
        .s3
        .get_object(&bucket, &key, q.version_id.as_deref(), range, sse_c_key)
        .await
    {
        Ok(obj) => {
            let status = if range.is_some() {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            };
            let mut resp = (status, obj.body).into_response();
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, obj.content_type.parse().unwrap_or(header::HeaderValue::from_static("application/octet-stream")));
            h.insert("ETag", format!("\"{}\"", obj.etag).parse().unwrap());
            h.insert(header::LAST_MODIFIED, obj.last_modified.format("%a, %d %b %Y %H:%M:%S GMT").to_string().parse().unwrap());
            h.insert(header::CONTENT_LENGTH, obj.size.to_string().parse().unwrap());
            for (k, v) in &obj.metadata {
                if let Ok(hv) = v.parse() {
                    h.insert(
                        format!("x-amz-meta-{k}").parse::<header::HeaderName>().unwrap(),
                        hv,
                    );
                }
            }
            if let Some(vid) = obj.version_id {
                h.insert("x-amz-version-id", vid.parse().unwrap());
            }
            if let Some(cr) = obj.content_range {
                h.insert(header::CONTENT_RANGE, cr.parse().unwrap());
            }
            resp
        }
        Err(e) => store_error_response(e, &format!("/{bucket}/{key}")),
    }
}

/// HEAD /{bucket}/{key+}
async fn head_object(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
) -> Response {
    match state.s3.head_object(&bucket, &key, q.version_id.as_deref()).await {
        Ok(obj) => {
            let mut resp = StatusCode::OK.into_response();
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, obj.content_type.parse().unwrap_or(header::HeaderValue::from_static("application/octet-stream")));
            h.insert("ETag", format!("\"{}\"", obj.etag).parse().unwrap());
            h.insert(header::LAST_MODIFIED, obj.last_modified.format("%a, %d %b %Y %H:%M:%S GMT").to_string().parse().unwrap());
            h.insert(header::CONTENT_LENGTH, obj.size.to_string().parse().unwrap());
            if let Some(vid) = obj.version_id {
                h.insert("x-amz-version-id", vid.parse().unwrap());
            }
            if obj.delete_marker {
                h.insert("x-amz-delete-marker", "true".parse().unwrap());
            }
            resp
        }
        Err(e) => StatusCode::from_u16(e.s3_status()).unwrap_or(StatusCode::NOT_FOUND).into_response(),
    }
}

/// DELETE /{bucket}/{key+}
async fn delete_object(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
) -> Response {
    // Abort multipart upload
    if let Some(upload_id) = &q.upload_id {
        return match state.s3.abort_multipart_upload(upload_id).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => store_error_response(e, &format!("/{bucket}/{key}?uploadId={upload_id}")),
        };
    }

    if q.tagging.is_some() {
        // Delete object tagging
        return match state.s3.put_object_tagging(&bucket, &key, q.version_id.as_deref(), HashMap::new()).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => store_error_response(e, &format!("/{bucket}/{key}?tagging")),
        };
    }

    match state.s3.delete_object(&bucket, &key, q.version_id.as_deref()).await {
        Ok(r) => {
            let mut resp = StatusCode::NO_CONTENT.into_response();
            if let Some(vid) = r.version_id {
                resp.headers_mut().insert("x-amz-version-id", vid.parse().unwrap());
            }
            if r.delete_marker {
                resp.headers_mut().insert("x-amz-delete-marker", "true".parse().unwrap());
            }
            resp
        }
        Err(e) => store_error_response(e, &format!("/{bucket}/{key}")),
    }
}

/// POST /{bucket}/{key+} — multipart upload operations or object tagging
async fn post_object(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Initiate multipart upload
    if q.uploads.is_some() {
        let content_type = headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let metadata = extract_user_metadata(&headers);
        return match state.s3.create_multipart_upload(&bucket, &key, &content_type, metadata).await {
            Ok(upload_id) => xml_response(
                StatusCode::OK,
                xml::initiate_multipart_upload(&bucket, &key, &upload_id),
            ),
            Err(e) => store_error_response(e, &format!("/{bucket}/{key}?uploads")),
        };
    }

    // Complete multipart upload
    if let Some(ref upload_id) = q.upload_id {
        // Parse the CompleteMultipartUpload XML body
        // Parts: <Part><PartNumber>N</PartNumber><ETag>"etag"</ETag></Part>
        let parts = parse_complete_multipart_body(&body);
        return match state.s3.complete_multipart_upload(upload_id, parts).await {
            Ok(r) => {
                let location = format!("/{}/{}", r.bucket, r.key);
                xml_response(
                    StatusCode::OK,
                    xml::complete_multipart_upload(&location, &r.bucket, &r.key, &r.etag, r.version_id.as_deref()),
                )
            }
            Err(e) => store_error_response(e, &format!("/{bucket}/{key}")),
        };
    }

    StatusCode::BAD_REQUEST.into_response()
}

/// POST /{bucket}?delete — delete multiple objects
async fn delete_objects_batch(
    State(state): State<Arc<StoreState>>,
    Path(bucket): Path<String>,
    body: Bytes,
) -> Response {
    let entries = parse_delete_objects_body(&body);
    match state.s3.delete_objects(&bucket, entries).await {
        Ok(results) => {
            let mut deleted = Vec::new();
            let mut errors = Vec::new();
            for r in results {
                if let Some((code, msg)) = r.error {
                    errors.push((r.key, code, msg));
                } else {
                    deleted.push(r.key);
                }
            }
            let dr = xml::DeleteResult { deleted, errors };
            xml_response(StatusCode::OK, xml::delete_objects_result(&dr))
        }
        Err(e) => store_error_response(e, &format!("/{bucket}?delete")),
    }
}

async fn get_object_tagging_handler(
    state: Arc<StoreState>,
    bucket: String,
    key: String,
    version_id: Option<&str>,
) -> Response {
    match state.s3.get_object_tagging(&bucket, &key, version_id).await {
        Ok(tags) => xml_response(StatusCode::OK, xml::get_object_tagging(&tags)),
        Err(e) => store_error_response(e, &format!("/{bucket}/{key}?tagging")),
    }
}

/// GET /{bucket}/{key+}?partNumber for listing parts
async fn list_parts_handler(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
) -> Response {
    if let Some(upload_id) = &q.upload_id {
        return match state.s3.list_parts(upload_id).await {
            Ok(parts) => {
                let items: Vec<xml::PartItem> = parts
                    .iter()
                    .map(|p| xml::PartItem {
                        part_number: p.part_number,
                        last_modified: p.last_modified,
                        etag: p.etag.clone(),
                        size: p.size,
                    })
                    .collect();
                xml_response(StatusCode::OK, xml::list_parts(&bucket, &key, upload_id, &items))
            }
            Err(e) => store_error_response(e, &format!("/{bucket}/{key}")),
        };
    }

    // List multipart uploads for bucket
    let uploads = state.s3.list_multipart_uploads(&bucket).await;
    let items: Vec<xml::MultipartUploadItem> = uploads
        .iter()
        .filter(|u| u.key == key)
        .map(|u| xml::MultipartUploadItem {
            key: u.key.clone(),
            upload_id: u.upload_id.clone(),
            initiated: u.initiated,
            owner_id: u.owner.clone(),
        })
        .collect();
    xml_response(StatusCode::OK, xml::list_multipart_uploads(&bucket, &items))
}

/// PUT /{bucket}/{key+}?tagging — put object tagging
async fn put_object_tagging(
    State(state): State<Arc<StoreState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<ObjectQuery>,
    body: Bytes,
) -> Response {
    let tags: HashMap<String, String> = serde_json::from_slice(&body).unwrap_or_default();
    match state.s3.put_object_tagging(&bucket, &key, q.version_id.as_deref(), tags).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => store_error_response(e, &format!("/{bucket}/{key}?tagging")),
    }
}

/// POST /presign — generate a presigned URL
#[derive(Debug, Deserialize)]
struct PresignRequest {
    bucket: String,
    key: String,
    method: String,
    expires_in: u64,
}

async fn generate_presigned_url(
    State(state): State<Arc<StoreState>>,
    axum::Json(req): axum::Json<PresignRequest>,
) -> axum::Json<serde_json::Value> {
    let params = presigned::PresignedUrlParams {
        bucket: req.bucket,
        key: req.key,
        method: req.method,
        expires_in_secs: req.expires_in,
        access_key: "cave-access-key".to_string(),
        extra_headers: HashMap::new(),
    };
    let url = presigned::generate("http://localhost:9000", &params, state.s3_secret_key.as_bytes());
    axum::Json(serde_json::json!({
        "url": url.url,
        "expires_at": url.expires_at,
    }))
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn s3_router(state: Arc<StoreState>) -> Router {
    Router::new()
        // Service-level
        .route("/", get(list_buckets))
        .route("/presign", post(generate_presigned_url))
        // Bucket-level
        .route("/:bucket", put(create_bucket))
        .route("/:bucket", delete(delete_bucket))
        .route("/:bucket", head(head_bucket))
        .route("/:bucket", get(get_bucket))
        .route("/:bucket", post(delete_objects_batch)) // ?delete
        // Object-level (key may contain slashes, use wildcard)
        .route("/:bucket/*key", put(put_object))
        .route("/:bucket/*key", get(get_object))
        .route("/:bucket/*key", head(head_object))
        .route("/:bucket/*key", delete(delete_object))
        .route("/:bucket/*key", post(post_object))
        .with_state(state)
}

// ── Parse helpers ─────────────────────────────────────────────────────────────

fn extract_user_metadata(headers: &HeaderMap) -> HashMap<String, String> {
    let mut meta = HashMap::new();
    for (name, value) in headers.iter() {
        let name_str = name.as_str();
        if let Some(key) = name_str.strip_prefix("x-amz-meta-") {
            if let Ok(v) = value.to_str() {
                meta.insert(key.to_string(), v.to_string());
            }
        }
    }
    meta
}

fn extract_tags(headers: &HeaderMap) -> HashMap<String, String> {
    let mut tags = HashMap::new();
    if let Some(tagging) = headers.get("x-amz-tagging").and_then(|v| v.to_str().ok()) {
        // URL-encoded key=value&key=value
        for pair in tagging.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                tags.insert(url_decode(k), url_decode(v));
            }
        }
    }
    tags
}

fn url_decode(s: &str) -> String {
    let mut out = String::new();
    let mut bytes = s.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let h1 = bytes.next().unwrap_or(b'0');
            let h2 = bytes.next().unwrap_or(b'0');
            if let Ok(c) = u8::from_str_radix(&format!("{}{}", h1 as char, h2 as char), 16) {
                out.push(c as char);
            }
        } else if b == b'+' {
            out.push(' ');
        } else {
            out.push(b as char);
        }
    }
    out
}

fn parse_copy_source(src: &str) -> (String, String) {
    let src = src.trim_start_matches('/');
    if let Some(idx) = src.find('/') {
        (src[..idx].to_string(), src[idx + 1..].to_string())
    } else {
        (src.to_string(), String::new())
    }
}

/// Parse `Range: bytes=start-end` header.
fn parse_range_header(range: Option<&str>) -> Option<(u64, u64)> {
    let range = range?;
    let range = range.strip_prefix("bytes=")?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.trim().parse().ok()?;
    let end: u64 = end.trim().parse().unwrap_or(u64::MAX);
    Some((start, end))
}

/// Parse CompleteMultipartUpload XML body.
/// Handles: <Part><PartNumber>N</PartNumber><ETag>"etag"</ETag></Part>
fn parse_complete_multipart_body(body: &[u8]) -> Vec<(u32, String)> {
    let body_str = String::from_utf8_lossy(body);
    let mut parts = Vec::new();
    // Simple regex-free XML parsing
    let mut remaining = body_str.as_ref();
    while let Some(part_start) = remaining.find("<Part>") {
        remaining = &remaining[part_start + "<Part>".len()..];
        let part_end = remaining.find("</Part>").unwrap_or(remaining.len());
        let part_xml = &remaining[..part_end];

        let pn = extract_xml_tag(part_xml, "PartNumber")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let etag = extract_xml_tag(part_xml, "ETag")
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_default();

        if pn > 0 {
            parts.push((pn, etag));
        }
        remaining = &remaining[part_end..];
    }
    parts
}

/// Parse Delete XML body.
fn parse_delete_objects_body(body: &[u8]) -> Vec<DeleteObjectEntry> {
    let body_str = String::from_utf8_lossy(body);
    let mut entries = Vec::new();
    let mut remaining = body_str.as_ref();
    while let Some(obj_start) = remaining.find("<Object>") {
        remaining = &remaining[obj_start + "<Object>".len()..];
        let obj_end = remaining.find("</Object>").unwrap_or(remaining.len());
        let obj_xml = &remaining[..obj_end];
        let key = extract_xml_tag(obj_xml, "Key").unwrap_or_default();
        let version_id = extract_xml_tag(obj_xml, "VersionId");
        if !key.is_empty() {
            entries.push(DeleteObjectEntry { key, version_id });
        }
        remaining = &remaining[obj_end..];
    }
    entries
}

fn extract_xml_tag<'a>(xml: &'a str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml.find(&close)?;
    if start <= end {
        Some(xml[start..end].to_string())
    } else {
        None
    }
}
