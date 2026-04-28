//! S3-compatible HTTP API surface (axum).
//!
//! Routes mirrored from the AWS S3 REST reference:
//!
//! | method | path                              | operation                |
//! |--------|-----------------------------------|--------------------------|
//! | PUT    | /:bucket                          | CreateBucket             |
//! | DELETE | /:bucket                          | DeleteBucket             |
//! | GET    | /:bucket?prefix=…                 | ListObjectsV2            |
//! | GET    | /:bucket/:key                     | GetObject                |
//! | HEAD   | /:bucket/:key                     | HeadObject               |
//! | PUT    | /:bucket/:key                     | PutObject                |
//! | DELETE | /:bucket/:key                     | DeleteObject             |
//! | POST   | /:bucket/:key?uploads             | CreateMultipartUpload    |
//! | PUT    | /:bucket/:key?partNumber=&uploadId| UploadPart               |
//! | POST   | /:bucket/:key?uploadId=           | CompleteMultipartUpload  |
//! | DELETE | /:bucket/:key?uploadId=           | AbortMultipartUpload     |

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, delete, get, head, post, put};
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

async fn bucket_handler(
    State(state): State<Arc<AppState>>,
    Path(bucket): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    method: axum::http::Method,
) -> Response {
    use axum::http::Method;
    match method {
        Method::PUT => create_bucket(&state, &bucket).await,
        Method::DELETE => delete_bucket(&state, &bucket).await,
        Method::GET => list_objects(&state, &bucket, &q).await,
        Method::HEAD => head_bucket(&state, &bucket).await,
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
    match method {
        Method::PUT if q.contains_key("partNumber") => upload_part(&state, &bucket, &key, &q, body).await,
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
}
