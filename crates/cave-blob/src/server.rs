//! S3-compatible HTTP API surface (axum).
//!
//! Routes mirrored from the AWS S3 REST reference:
//!
//! | method | path                              | operation                |
//! |--------|-----------------------------------|--------------------------|
//! | GET    | /                                 | ListBuckets              |
//! | PUT    | /:bucket                          | CreateBucket             |
//! | DELETE | /:bucket                          | DeleteBucket             |
//! | GET    | /:bucket?prefix=…                 | ListObjectsV2            |
//! | GET    | /:bucket?location                 | GetBucketLocation        |
//! | GET    | /:bucket?versioning               | GetBucketVersioning      |
//! | PUT    | /:bucket?versioning               | PutBucketVersioning      |
//! | POST   | /:bucket?delete                   | DeleteObjects            |
//! | GET    | /:bucket/:key                     | GetObject                |
//! | HEAD   | /:bucket/:key                     | HeadObject               |
//! | PUT    | /:bucket/:key                     | PutObject                |
//! | PUT    | /:bucket/:key (x-amz-copy-source) | CopyObject               |
//! | PUT    | /:bucket/:key?partNumber&uploadId | UploadPart               |
//! | PUT    | /:b/:k?partNumber&uploadId        | UploadPartCopy           |
//! |        |   (with x-amz-copy-source)        |                          |
//! | DELETE | /:bucket/:key                     | DeleteObject             |
//! | POST   | /:bucket/:key?uploads             | CreateMultipartUpload    |
//! | POST   | /:bucket/:key?uploadId=           | CompleteMultipartUpload  |
//! | DELETE | /:bucket/:key?uploadId=           | AbortMultipartUpload     |

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use chrono::Utc;

use crate::error::{BlobError, BlobResult};
use crate::lock::{ObjectLockMode, ObjectLockRetention};
use crate::multipart::UploadRegistry;
use crate::object::{compute_etag, Object, ObjectMeta};
use crate::store::{BucketMeta, Store};

#[derive(Debug)]
pub struct AppState {
    pub store: Store,
    pub uploads: UploadRegistry,
    pub default_tenant: String,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            store: Store::new(),
            uploads: UploadRegistry::new(),
            default_tenant: "default".into(),
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", any(root_handler))
        .route("/{bucket}", any(bucket_handler))
        .route("/{bucket}/", any(bucket_handler))
        .route("/{bucket}/{*key}", any(object_handler))
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

// ── Handlers ────────────────────────────────────────────────────────────────

async fn root_handler(
    State(state): State<Arc<AppState>>,
    method: axum::http::Method,
) -> Response {
    use axum::http::Method;
    match method {
        Method::GET => list_buckets(&state).await,
        _ => not_implemented(),
    }
}

async fn bucket_handler(
    State(state): State<Arc<AppState>>,
    Path(bucket): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    method: axum::http::Method,
    body: Bytes,
) -> Response {
    use axum::http::Method;
    match method {
        Method::PUT if q.contains_key("versioning") => {
            put_bucket_versioning(&state, &bucket, &body).await
        }
        Method::PUT => create_bucket(&state, &bucket).await,
        Method::DELETE => delete_bucket(&state, &bucket).await,
        Method::GET if q.contains_key("location") => get_bucket_location(&state, &bucket).await,
        Method::GET if q.contains_key("versioning") => {
            get_bucket_versioning(&state, &bucket).await
        }
        Method::GET => list_objects(&state, &bucket, &q).await,
        Method::HEAD => head_bucket(&state, &bucket).await,
        Method::POST if q.contains_key("delete") => delete_objects(&state, &bucket, &body).await,
        _ => not_implemented(),
    }
}

async fn object_handler(
    State(state): State<Arc<AppState>>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    method: axum::http::Method,
    body: Bytes,
) -> Response {
    use axum::http::Method;
    let copy_source = headers
        .get("x-amz-copy-source")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    match method {
        Method::PUT if q.contains_key("partNumber") && copy_source.is_some() => {
            upload_part_copy(&state, copy_source.as_deref().unwrap_or(""), &q).await
        }
        Method::PUT if q.contains_key("partNumber") => {
            upload_part(&state, &bucket, &key, &q, body).await
        }
        Method::PUT if copy_source.is_some() => {
            copy_object(&state, &bucket, &key, copy_source.as_deref().unwrap_or("")).await
        }
        Method::PUT => put_object(&state, &bucket, &key, headers, body).await,
        Method::GET => get_object(&state, &bucket, &key).await,
        Method::HEAD => head_object(&state, &bucket, &key).await,
        Method::DELETE if q.contains_key("uploadId") => abort_multipart(&state, &q).await,
        Method::DELETE => delete_object(&state, &bucket, &key).await,
        Method::POST if q.contains_key("uploads") => create_multipart(&state, &bucket, &key).await,
        Method::POST if q.contains_key("uploadId") => complete_multipart(&state, &bucket, &key, &q, body).await,
        _ => not_implemented(),
    }
}

async fn create_bucket(state: &AppState, bucket: &str) -> Response {
    let result = state.store.create_bucket(BucketMeta {
        name: bucket.into(),
        tenant: state.default_tenant.clone(),
        created_at: Utc::now(),
        versioning_enabled: false,
        immutability_default: None,
    });
    blob_result(result.map(|_| ok_no_body()))
}

async fn delete_bucket(state: &AppState, bucket: &str) -> Response {
    blob_result(state.store.delete_bucket(bucket).map(|_| no_content()))
}

async fn head_bucket(state: &AppState, bucket: &str) -> Response {
    blob_result(state.store.bucket_meta(bucket).map(|_| ok_no_body()))
}

async fn list_buckets(state: &AppState) -> Response {
    let buckets = state.store.list_buckets();
    xml_response(render_list_buckets_xml(&buckets))
}

async fn get_bucket_location(state: &AppState, bucket: &str) -> Response {
    match state.store.bucket_meta(bucket) {
        Ok(_) => xml_response(render_bucket_location_xml()),
        Err(e) => blob_error(e),
    }
}

async fn get_bucket_versioning(state: &AppState, bucket: &str) -> Response {
    match state.store.bucket_meta(bucket) {
        Ok(meta) => xml_response(render_versioning_xml(meta.versioning_enabled)),
        Err(e) => blob_error(e),
    }
}

async fn put_bucket_versioning(state: &AppState, bucket: &str, body: &Bytes) -> Response {
    let xml = std::str::from_utf8(body).unwrap_or("");
    let enabled = parse_versioning_xml(xml);
    blob_result(state.store.set_versioning(bucket, enabled).map(|_| ok_no_body()))
}

async fn list_objects(state: &AppState, bucket: &str, q: &HashMap<String, String>) -> Response {
    let prefix = q.get("prefix").cloned().unwrap_or_default();
    match state.store.list_objects(bucket, &prefix) {
        Ok(metas) => xml_response(render_list_xml(bucket, &prefix, &metas)),
        Err(e) => blob_error(e),
    }
}

async fn put_object(state: &AppState, bucket: &str, key: &str, headers: HeaderMap, body: Bytes) -> Response {
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let mut obj = Object::new(bucket, key, body.to_vec(), &content_type);
    if let Some(lock_mode) = headers.get("x-amz-object-lock-mode").and_then(|v| v.to_str().ok()) {
        if let Some(mode) = ObjectLockMode::parse(lock_mode) {
            obj.meta.headers.insert("x-amz-object-lock-mode".into(), mode.as_str().into());
        }
    }
    if let Some(until) = headers.get("x-amz-object-lock-retain-until-date").and_then(|v| v.to_str().ok()) {
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(until) {
            obj.meta.immutable_until = Some(parsed.with_timezone(&Utc));
        }
    }
    match state.store.put_object(obj.clone()) {
        Ok(meta) => Response::builder()
            .status(StatusCode::OK)
            .header("ETag", format!("\"{}\"", meta.etag))
            .body(Body::empty())
            .unwrap(),
        Err(e) => blob_error(e),
    }
}

async fn copy_object(state: &AppState, dst_bucket: &str, dst_key: &str, copy_source: &str) -> Response {
    let (src_bucket, src_key) = match parse_copy_source(copy_source) {
        Some(p) => p,
        None => return bad_request("invalid x-amz-copy-source"),
    };
    let src_obj = match state.store.get_object(&src_bucket, &src_key, None) {
        Ok(o) => o,
        Err(e) => return blob_error(e),
    };
    let mut dst = Object::new(dst_bucket, dst_key, src_obj.body, &src_obj.meta.content_type);
    dst.meta.headers = src_obj.meta.headers.clone();
    match state.store.put_object(dst) {
        Ok(meta) => xml_response(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <CopyObjectResult>\n  \
             <ETag>\"{}\"</ETag>\n  \
             <LastModified>{}</LastModified>\n\
             </CopyObjectResult>",
            meta.etag,
            meta.last_modified.to_rfc3339()
        )),
        Err(e) => blob_error(e),
    }
}

async fn upload_part_copy(state: &AppState, copy_source: &str, q: &HashMap<String, String>) -> Response {
    let upload_id = match q.get("uploadId") {
        Some(s) => s.clone(),
        None => return bad_request("uploadId required"),
    };
    let part_number = match q.get("partNumber").and_then(|s| s.parse::<i32>().ok()) {
        Some(n) => n,
        None => return bad_request("partNumber required"),
    };
    let (src_bucket, src_key) = match parse_copy_source(copy_source) {
        Some(p) => p,
        None => return bad_request("invalid x-amz-copy-source"),
    };
    let src_obj = match state.store.get_object(&src_bucket, &src_key, None) {
        Ok(o) => o,
        Err(e) => return blob_error(e),
    };
    match state.uploads.upload_part(&upload_id, part_number, src_obj.body) {
        Ok(etag) => xml_response(format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <CopyPartResult>\n  \
             <ETag>\"{}\"</ETag>\n  \
             <LastModified>{}</LastModified>\n\
             </CopyPartResult>",
            etag,
            Utc::now().to_rfc3339()
        )),
        Err(e) => blob_error(e),
    }
}

async fn delete_objects(state: &AppState, bucket: &str, body: &Bytes) -> Response {
    let xml = std::str::from_utf8(body).unwrap_or("");
    let req = match parse_delete_objects_xml(xml) {
        Ok(r) => r,
        Err(_) => return bad_request("invalid Delete body"),
    };
    let now = Utc::now();
    let mut deleted: Vec<String> = Vec::new();
    let mut errors: Vec<(String, String, String)> = Vec::new(); // (key, code, message)
    for key in &req {
        match state.store.delete_object(bucket, key, None, now) {
            Ok(()) => deleted.push(key.clone()),
            Err(BlobError::ObjectNotFound(_, _)) => {
                // S3 returns deleted markers for not-found targets.
                deleted.push(key.clone());
            }
            Err(BlobError::BucketNotFound(b)) => {
                return blob_error(BlobError::BucketNotFound(b));
            }
            Err(e) => {
                let code = match &e {
                    BlobError::Immutable => "InvalidRetentionMode".to_string(),
                    BlobError::TenantDenied(_) => "AccessDenied".to_string(),
                    _ => "InternalError".to_string(),
                };
                errors.push((key.clone(), code, e.to_string()));
            }
        }
    }
    xml_response(render_delete_result_xml(&deleted, &errors))
}

async fn get_object(state: &AppState, bucket: &str, key: &str) -> Response {
    match state.store.get_object(bucket, key, None) {
        Ok(obj) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", &obj.meta.content_type)
            .header("Content-Length", obj.body.len().to_string())
            .header("ETag", format!("\"{}\"", obj.meta.etag))
            .body(Body::from(obj.body))
            .unwrap(),
        Err(e) => blob_error(e),
    }
}

async fn head_object(state: &AppState, bucket: &str, key: &str) -> Response {
    match state.store.head_object(bucket, key, None) {
        Ok(meta) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", &meta.content_type)
            .header("Content-Length", meta.size.to_string())
            .header("ETag", format!("\"{}\"", meta.etag))
            .body(Body::empty())
            .unwrap(),
        Err(e) => blob_error(e),
    }
}

async fn delete_object(state: &AppState, bucket: &str, key: &str) -> Response {
    match state.store.delete_object(bucket, key, None, Utc::now()) {
        Ok(()) => no_content(),
        Err(e) => blob_error(e),
    }
}

async fn create_multipart(state: &AppState, bucket: &str, key: &str) -> Response {
    let upload_id = state.uploads.initiate(bucket, key);
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <InitiateMultipartUploadResult>\n  \
         <Bucket>{}</Bucket>\n  <Key>{}</Key>\n  <UploadId>{}</UploadId>\n\
         </InitiateMultipartUploadResult>",
        bucket, key, upload_id
    );
    xml_response(xml)
}

async fn upload_part(
    state: &AppState,
    _bucket: &str,
    _key: &str,
    q: &HashMap<String, String>,
    body: Bytes,
) -> Response {
    let upload_id = match q.get("uploadId") {
        Some(s) => s.clone(),
        None => return bad_request("uploadId required"),
    };
    let part_number = match q.get("partNumber").and_then(|s| s.parse::<i32>().ok()) {
        Some(n) => n,
        None => return bad_request("partNumber required"),
    };
    match state.uploads.upload_part(&upload_id, part_number, body.to_vec()) {
        Ok(etag) => Response::builder()
            .status(StatusCode::OK)
            .header("ETag", format!("\"{}\"", etag))
            .body(Body::empty())
            .unwrap(),
        Err(e) => blob_error(e),
    }
}

async fn complete_multipart(
    state: &AppState,
    bucket: &str,
    key: &str,
    q: &HashMap<String, String>,
    body: Bytes,
) -> Response {
    let upload_id = match q.get("uploadId") {
        Some(s) => s.clone(),
        None => return bad_request("uploadId required"),
    };
    let parts = match parse_complete_multipart_xml(std::str::from_utf8(&body).unwrap_or("")) {
        Ok(parts) => parts,
        Err(_) => return bad_request("invalid CompleteMultipartUpload body"),
    };
    match state.uploads.complete(&upload_id, &parts, "application/octet-stream") {
        Ok(obj) => match state.store.put_object(obj.clone()) {
            Ok(meta) => xml_response(format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <CompleteMultipartUploadResult>\n  \
                 <Location>http://example.com/{}/{}</Location>\n  \
                 <Bucket>{}</Bucket>\n  <Key>{}</Key>\n  <ETag>\"{}\"</ETag>\n\
                 </CompleteMultipartUploadResult>",
                bucket, key, bucket, key, meta.etag,
            )),
            Err(e) => blob_error(e),
        },
        Err(e) => blob_error(e),
    }
}

async fn abort_multipart(state: &AppState, q: &HashMap<String, String>) -> Response {
    let upload_id = match q.get("uploadId") {
        Some(s) => s.clone(),
        None => return bad_request("uploadId required"),
    };
    match state.uploads.abort(&upload_id) {
        Ok(()) => no_content(),
        Err(e) => blob_error(e),
    }
}

// ── XML helpers ─────────────────────────────────────────────────────────────

fn render_list_xml(bucket: &str, prefix: &str, metas: &[ObjectMeta]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<ListBucketResult>\n");
    out.push_str(&format!("  <Name>{}</Name>\n", bucket));
    out.push_str(&format!("  <Prefix>{}</Prefix>\n", prefix));
    out.push_str(&format!("  <KeyCount>{}</KeyCount>\n", metas.len()));
    for m in metas {
        out.push_str("  <Contents>\n");
        out.push_str(&format!("    <Key>{}</Key>\n", m.key));
        out.push_str(&format!("    <Size>{}</Size>\n", m.size));
        out.push_str(&format!("    <ETag>\"{}\"</ETag>\n", m.etag));
        out.push_str(&format!(
            "    <LastModified>{}</LastModified>\n",
            m.last_modified.to_rfc3339()
        ));
        out.push_str("  </Contents>\n");
    }
    out.push_str("</ListBucketResult>");
    out
}

fn render_list_buckets_xml(buckets: &[BucketMeta]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<ListAllMyBucketsResult>\n");
    out.push_str("  <Owner>\n    <ID>cave</ID>\n    <DisplayName>cave</DisplayName>\n  </Owner>\n");
    out.push_str("  <Buckets>\n");
    for b in buckets {
        out.push_str("    <Bucket>\n");
        out.push_str(&format!("      <Name>{}</Name>\n", b.name));
        out.push_str(&format!(
            "      <CreationDate>{}</CreationDate>\n",
            b.created_at.to_rfc3339()
        ));
        out.push_str("    </Bucket>\n");
    }
    out.push_str("  </Buckets>\n");
    out.push_str("</ListAllMyBucketsResult>");
    out
}

fn render_bucket_location_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
     <LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">us-east-1</LocationConstraint>"
        .to_string()
}

fn render_versioning_xml(enabled: bool) -> String {
    let status = if enabled { "Enabled" } else { "Suspended" };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n  \
         <Status>{}</Status>\n\
         </VersioningConfiguration>",
        status
    )
}

/// Parse `<VersioningConfiguration><Status>Enabled|Suspended</Status>...`.
pub fn parse_versioning_xml(xml: &str) -> bool {
    if let Ok(s) = extract_xml_field(xml, "Status") {
        s.eq_ignore_ascii_case("Enabled")
    } else {
        false
    }
}

/// Parse the `Delete` request body into a list of object keys to delete.
/// Quiet/Versioned variants are accepted but only Key is honored.
pub fn parse_delete_objects_xml(xml: &str) -> Result<Vec<String>, &'static str> {
    let mut out = Vec::new();
    let mut cursor = xml;
    while let Some(start) = cursor.find("<Object>") {
        cursor = &cursor[start + 8..];
        let end = cursor.find("</Object>").ok_or("unterminated Object")?;
        let chunk = &cursor[..end];
        let key = extract_xml_field(chunk, "Key")?.to_string();
        out.push(key);
        cursor = &cursor[end + 9..];
    }
    Ok(out)
}

fn render_delete_result_xml(deleted: &[String], errors: &[(String, String, String)]) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<DeleteResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">\n");
    for k in deleted {
        out.push_str("  <Deleted>\n");
        out.push_str(&format!("    <Key>{}</Key>\n", k));
        out.push_str("  </Deleted>\n");
    }
    for (k, code, msg) in errors {
        out.push_str("  <Error>\n");
        out.push_str(&format!("    <Key>{}</Key>\n", k));
        out.push_str(&format!("    <Code>{}</Code>\n", code));
        out.push_str(&format!("    <Message>{}</Message>\n", msg));
        out.push_str("  </Error>\n");
    }
    out.push_str("</DeleteResult>");
    out
}

/// Parse `x-amz-copy-source` header; supports the two AWS forms:
///   `bucket/key`        — relative
///   `/bucket/key`       — leading slash
/// Returns `None` if the header is malformed.
pub fn parse_copy_source(header: &str) -> Option<(String, String)> {
    let trimmed = header.trim_start_matches('/');
    let (bucket, key) = trimmed.split_once('/')?;
    if bucket.is_empty() || key.is_empty() {
        return None;
    }
    Some((bucket.to_string(), key.to_string()))
}

pub fn parse_complete_multipart_xml(xml: &str) -> Result<Vec<(i32, String)>, &'static str> {
    let mut out = Vec::new();
    let mut cursor = xml;
    while let Some(start) = cursor.find("<Part>") {
        cursor = &cursor[start + 6..];
        let end = cursor.find("</Part>").ok_or("unterminated Part")?;
        let chunk = &cursor[..end];
        let part_number = extract_xml_field(chunk, "PartNumber")?
            .parse::<i32>()
            .map_err(|_| "invalid PartNumber")?;
        let etag = extract_xml_field(chunk, "ETag")?
            .trim_matches('"')
            .to_string();
        out.push((part_number, etag));
        cursor = &cursor[end + 7..];
    }
    Ok(out)
}

fn extract_xml_field<'a>(chunk: &'a str, tag: &str) -> Result<&'a str, &'static str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let s = chunk.find(&open).ok_or("missing tag")?;
    let e = chunk.find(&close).ok_or("missing close tag")?;
    Ok(&chunk[s + open.len()..e])
}

// ── Response helpers ────────────────────────────────────────────────────────

fn ok_no_body() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::empty())
        .unwrap()
}

fn no_content() -> Response {
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .unwrap()
}

fn xml_response(body: String) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/xml")
        .body(Body::from(body))
        .unwrap()
}

fn bad_request(msg: &'static str) -> Response {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(Body::from(msg))
        .unwrap()
}

fn not_implemented() -> Response {
    Response::builder()
        .status(StatusCode::METHOD_NOT_ALLOWED)
        .body(Body::empty())
        .unwrap()
}

fn blob_error(e: BlobError) -> Response {
    let (code, text) = match &e {
        BlobError::BucketNotFound(_) => (StatusCode::NOT_FOUND, "NoSuchBucket"),
        BlobError::BucketExists(_) => (StatusCode::CONFLICT, "BucketAlreadyExists"),
        BlobError::ObjectNotFound(_, _) => (StatusCode::NOT_FOUND, "NoSuchKey"),
        BlobError::InvalidKey(_) | BlobError::InvalidBucketName(_) => {
            (StatusCode::BAD_REQUEST, "InvalidArgument")
        }
        BlobError::UploadNotFound(_) => (StatusCode::NOT_FOUND, "NoSuchUpload"),
        BlobError::EtagMismatch(_, _, _) => (StatusCode::BAD_REQUEST, "InvalidPart"),
        BlobError::Immutable => (StatusCode::FORBIDDEN, "InvalidRetentionMode"),
        BlobError::TenantDenied(_) => (StatusCode::FORBIDDEN, "AccessDenied"),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "InternalError"),
    };
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <Error><Code>{}</Code><Message>{}</Message></Error>",
        text, e
    );
    Response::builder()
        .status(code)
        .header("Content-Type", "application/xml")
        .body(Body::from(body))
        .unwrap()
}

fn blob_result<T: IntoResponse>(r: BlobResult<T>) -> Response {
    match r {
        Ok(v) => v.into_response(),
        Err(e) => blob_error(e),
    }
}

// Keep imports referenced.
#[allow(dead_code)]
fn _keep() {
    let _ = compute_etag;
    let _: ObjectLockRetention = ObjectLockRetention::Compliance;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_complete_multipart_xml_one_part() {
        let xml = "<Part><PartNumber>1</PartNumber><ETag>\"abc\"</ETag></Part>";
        let parts = parse_complete_multipart_xml(xml).unwrap();
        assert_eq!(parts, vec![(1, "abc".to_string())]);
    }

    #[test]
    fn parse_complete_multipart_xml_multiple_parts() {
        let xml = "<Part><PartNumber>1</PartNumber><ETag>\"a\"</ETag></Part>\
                   <Part><PartNumber>2</PartNumber><ETag>\"b\"</ETag></Part>";
        let parts = parse_complete_multipart_xml(xml).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1], (2, "b".to_string()));
    }

    #[test]
    fn parse_complete_multipart_xml_empty() {
        let parts = parse_complete_multipart_xml("").unwrap();
        assert!(parts.is_empty());
    }

    #[test]
    fn render_list_xml_includes_prefix_and_count() {
        let xml = render_list_xml("b", "logs/", &[]);
        assert!(xml.contains("<Name>b</Name>"));
        assert!(xml.contains("<Prefix>logs/</Prefix>"));
        assert!(xml.contains("<KeyCount>0</KeyCount>"));
    }

    #[test]
    fn render_list_xml_lists_objects() {
        let m = ObjectMeta {
            bucket: "b".into(),
            key: "k".into(),
            etag: "abc".into(),
            size: 42,
            content_type: "text/plain".into(),
            last_modified: Utc::now(),
            headers: Default::default(),
            version_id: None,
            immutable_until: None,
        };
        let xml = render_list_xml("b", "", &[m]);
        assert!(xml.contains("<Key>k</Key>"));
        assert!(xml.contains("<Size>42</Size>"));
    }

    #[test]
    fn extract_xml_field_simple() {
        let s = "<X>42</X>";
        assert_eq!(extract_xml_field(s, "X").unwrap(), "42");
    }

    #[test]
    fn extract_xml_field_missing_errors() {
        let s = "<X>1</X>";
        assert!(extract_xml_field(s, "Y").is_err());
    }

    // ── New ops ──────────────────────────────────────────────────────────

    #[test]
    fn parse_copy_source_relative() {
        let p = parse_copy_source("src-bucket/path/key.txt").unwrap();
        assert_eq!(p, ("src-bucket".into(), "path/key.txt".into()));
    }

    #[test]
    fn parse_copy_source_leading_slash() {
        let p = parse_copy_source("/src-bucket/k").unwrap();
        assert_eq!(p, ("src-bucket".into(), "k".into()));
    }

    #[test]
    fn parse_copy_source_invalid_returns_none() {
        assert!(parse_copy_source("nokey").is_none());
        assert!(parse_copy_source("/").is_none());
    }

    #[test]
    fn parse_delete_objects_xml_one_key() {
        let xml = "<Delete><Object><Key>a.txt</Key></Object></Delete>";
        let keys = parse_delete_objects_xml(xml).unwrap();
        assert_eq!(keys, vec!["a.txt"]);
    }

    #[test]
    fn parse_delete_objects_xml_multiple() {
        let xml = "<Delete>\
                   <Object><Key>a</Key></Object>\
                   <Object><Key>b/c</Key></Object>\
                   </Delete>";
        let keys = parse_delete_objects_xml(xml).unwrap();
        assert_eq!(keys, vec!["a", "b/c"]);
    }

    #[test]
    fn parse_delete_objects_xml_empty_ok() {
        assert!(parse_delete_objects_xml("<Delete/>").unwrap().is_empty());
    }

    #[test]
    fn parse_versioning_xml_enabled() {
        let xml = "<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>";
        assert!(parse_versioning_xml(xml));
    }

    #[test]
    fn parse_versioning_xml_suspended() {
        let xml = "<VersioningConfiguration><Status>Suspended</Status></VersioningConfiguration>";
        assert!(!parse_versioning_xml(xml));
    }

    #[test]
    fn render_versioning_xml_enabled_string() {
        assert!(render_versioning_xml(true).contains("<Status>Enabled</Status>"));
        assert!(render_versioning_xml(false).contains("<Status>Suspended</Status>"));
    }

    #[test]
    fn render_list_buckets_xml_empty() {
        let s = render_list_buckets_xml(&[]);
        assert!(s.contains("<ListAllMyBucketsResult>"));
        assert!(s.contains("<Buckets>"));
    }

    #[test]
    fn render_list_buckets_xml_with_buckets() {
        let m = BucketMeta {
            name: "bk".into(),
            tenant: "default".into(),
            created_at: Utc::now(),
            versioning_enabled: false,
            immutability_default: None,
        };
        let s = render_list_buckets_xml(&[m]);
        assert!(s.contains("<Name>bk</Name>"));
        assert!(s.contains("<CreationDate>"));
    }

    #[test]
    fn render_bucket_location_default_region() {
        let s = render_bucket_location_xml();
        assert!(s.contains("us-east-1"));
        assert!(s.contains("LocationConstraint"));
    }

    #[test]
    fn render_delete_result_emits_deleted_and_error() {
        let xml = render_delete_result_xml(
            &["a".into(), "b".into()],
            &[("c".into(), "AccessDenied".into(), "denied".into())],
        );
        assert!(xml.contains("<Deleted>"));
        assert!(xml.contains("<Key>a</Key>"));
        assert!(xml.contains("<Error>"));
        assert!(xml.contains("<Code>AccessDenied</Code>"));
    }

    // ── Wired-handler smoke tests via tower::ServiceExt ─────────────────

    #[tokio::test]
    async fn list_buckets_returns_xml() {
        use tower::ServiceExt;
        let state = AppState::new();
        state
            .store
            .create_bucket(BucketMeta {
                name: "alpha".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        let app = router(state);
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let s = String::from_utf8_lossy(&bytes).to_string();
        assert!(s.contains("<Name>alpha</Name>"));
    }

    #[tokio::test]
    async fn delete_objects_round_trip() {
        use tower::ServiceExt;
        let state = AppState::new();
        state
            .store
            .create_bucket(BucketMeta {
                name: "buk".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        state
            .store
            .put_object(Object::new("buk", "a", b"x".to_vec(), "t"))
            .unwrap();
        state
            .store
            .put_object(Object::new("buk", "b", b"y".to_vec(), "t"))
            .unwrap();
        let body = "<Delete><Object><Key>a</Key></Object>\
                    <Object><Key>b</Key></Object></Delete>";
        let app = router(state.clone());
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/buk?delete")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let s = String::from_utf8_lossy(&bytes).to_string();
        assert!(s.contains("<Key>a</Key>"));
        assert!(s.contains("<Key>b</Key>"));
        // store should now be empty
        assert!(state.store.list_objects("buk", "").unwrap().is_empty());
    }

    #[tokio::test]
    async fn copy_object_creates_destination() {
        use tower::ServiceExt;
        let state = AppState::new();
        state
            .store
            .create_bucket(BucketMeta {
                name: "src".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        state
            .store
            .create_bucket(BucketMeta {
                name: "dst".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        state
            .store
            .put_object(Object::new("src", "k.txt", b"payload".to_vec(), "text/plain"))
            .unwrap();
        let app = router(state.clone());
        let req = axum::http::Request::builder()
            .method("PUT")
            .uri("/dst/copied.txt")
            .header("x-amz-copy-source", "src/k.txt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = state.store.get_object("dst", "copied.txt", None).unwrap();
        assert_eq!(body.body, b"payload");
    }

    #[tokio::test]
    async fn get_bucket_location_returns_region() {
        use tower::ServiceExt;
        let state = AppState::new();
        state
            .store
            .create_bucket(BucketMeta {
                name: "buk".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        let app = router(state);
        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/buk?location")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("us-east-1"));
    }

    #[tokio::test]
    async fn put_then_get_bucket_versioning() {
        use tower::ServiceExt;
        let state = AppState::new();
        state
            .store
            .create_bucket(BucketMeta {
                name: "buk".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        let app = router(state.clone());
        let body = "<VersioningConfiguration><Status>Enabled</Status></VersioningConfiguration>";
        let req = axum::http::Request::builder()
            .method("PUT")
            .uri("/buk?versioning")
            .body(Body::from(body))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(state.store.bucket_meta("buk").unwrap().versioning_enabled);

        let req2 = axum::http::Request::builder()
            .method("GET")
            .uri("/buk?versioning")
            .body(Body::empty())
            .unwrap();
        let resp2 = app.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp2.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("Enabled"));
    }

    #[tokio::test]
    async fn upload_part_copy_returns_part_etag() {
        use tower::ServiceExt;
        let state = AppState::new();
        state
            .store
            .create_bucket(BucketMeta {
                name: "src".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        state
            .store
            .create_bucket(BucketMeta {
                name: "dst".into(),
                tenant: "default".into(),
                created_at: Utc::now(),
                versioning_enabled: false,
                immutability_default: None,
            })
            .unwrap();
        state
            .store
            .put_object(Object::new("src", "src.bin", b"abcdef".to_vec(), "x"))
            .unwrap();
        let upload_id = state.uploads.initiate("dst", "out.bin");

        let app = router(state.clone());
        let req = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("/dst/out.bin?partNumber=1&uploadId={}", upload_id))
            .header("x-amz-copy-source", "src/src.bin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8_lossy(&bytes).to_string();
        assert!(body.contains("<CopyPartResult>"));
        let parts = state.uploads.list_parts(&upload_id).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].size, 6);
    }
}
