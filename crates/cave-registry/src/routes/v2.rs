//! Docker Registry V2 HTTP API handlers.
//!
//! All paths under /v2/* are dispatched here.  Repository names may contain
//! forward-slashes (e.g. "library/nginx"), so we use a single wildcard
//! extractor and parse the path manually.

use crate::{
    error::RegistryError,
    store::compute_digest,
    types::{
        CatalogResponse, TagsListResponse, MEDIA_MANIFEST_LIST, MEDIA_MANIFEST_V2,
        MEDIA_OCI_INDEX, MEDIA_OCI_MANIFEST,
    },
    AppState,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::sync::Arc;

// ── Path parsing ──────────────────────────────────────────────────────────────

#[derive(Debug)]
enum V2Action {
    Catalog,
    TagsList { name: String },
    Manifest { name: String, reference: String },
    BlobGet { name: String, digest: String },
    UploadStart { name: String },
    UploadChunk { name: String, session_id: String },
}

fn parse_path(path: &str) -> Option<V2Action> {
    let path = path.trim_start_matches('/');

    if path == "_catalog" {
        return Some(V2Action::Catalog);
    }
    if let Some(idx) = path.rfind("/manifests/") {
        let name = path[..idx].to_string();
        let reference = path[idx + "/manifests/".len()..].to_string();
        if !name.is_empty() && !reference.is_empty() {
            return Some(V2Action::Manifest { name, reference });
        }
    }
    if path.ends_with("/tags/list") {
        let name = path[..path.len() - "/tags/list".len()].to_string();
        if !name.is_empty() {
            return Some(V2Action::TagsList { name });
        }
    }
    // uploads with session: /name/blobs/uploads/<uuid>
    if let Some(idx) = path.rfind("/blobs/uploads/") {
        let name = path[..idx].to_string();
        let rest = path[idx + "/blobs/uploads/".len()..].to_string();
        if !name.is_empty() {
            if rest.is_empty() {
                return Some(V2Action::UploadStart { name });
            } else {
                return Some(V2Action::UploadChunk { name, session_id: rest });
            }
        }
    }
    // blobs by digest: /name/blobs/sha256:...
    if let Some(idx) = path.rfind("/blobs/") {
        let name = path[..idx].to_string();
        let digest = path[idx + "/blobs/".len()..].to_string();
        if !name.is_empty() && !digest.is_empty() {
            return Some(V2Action::BlobGet { name, digest });
        }
    }
    None
}

fn is_manifest_media_type(mt: &str) -> bool {
    matches!(
        mt,
        MEDIA_MANIFEST_V2 | MEDIA_MANIFEST_LIST | MEDIA_OCI_MANIFEST | MEDIA_OCI_INDEX
    )
}

fn extract_content_type(headers: &HeaderMap) -> String {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or(MEDIA_MANIFEST_V2)
        .to_string()
}

// ── Query params ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct PaginationQuery {
    pub n: Option<usize>,
    pub last: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DigestQuery {
    pub digest: Option<String>,
}

// ── V2 base check ─────────────────────────────────────────────────────────────

pub async fn v2_check() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            HeaderName::from_static("docker-distribution-api-version"),
            HeaderValue::from_static("registry/2.0"),
        )],
        "",
    )
}

// ── Wildcard dispatcher ───────────────────────────────────────────────────────

pub async fn v2_dispatch(
    method: Method,
    Path(path): Path<String>,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(digest_q): Query<DigestQuery>,
    body: Bytes,
) -> Response {
    let action = match parse_path(&path) {
        Some(a) => a,
        None => return RegistryError::UnsupportedPath.into_response(),
    };

    match action {
        V2Action::Catalog => handle_catalog(state).await,
        V2Action::TagsList { name } => handle_tags_list(state, name).await,
        V2Action::Manifest { name, reference } => match method {
            Method::GET => handle_get_manifest(state, name, reference).await,
            Method::HEAD => handle_head_manifest(state, name, reference).await,
            Method::PUT => {
                handle_put_manifest(state, name, reference, headers, body).await
            }
            Method::DELETE => handle_delete_manifest(state, name, reference).await,
            _ => RegistryError::MethodNotAllowed.into_response(),
        },
        V2Action::BlobGet { name, digest } => match method {
            Method::GET => handle_get_blob(state, name, digest).await,
            Method::HEAD => handle_head_blob(state, name, digest).await,
            Method::DELETE => handle_delete_blob(state, name, digest).await,
            _ => RegistryError::MethodNotAllowed.into_response(),
        },
        V2Action::UploadStart { name } => match method {
            Method::POST => handle_start_upload(state, name, digest_q, body).await,
            _ => RegistryError::MethodNotAllowed.into_response(),
        },
        V2Action::UploadChunk { name, session_id } => match method {
            Method::PATCH => handle_patch_upload(state, name, session_id, body).await,
            Method::PUT => {
                handle_complete_upload(state, name, session_id, digest_q, body).await
            }
            Method::DELETE => handle_cancel_upload(state, name, session_id).await,
            _ => RegistryError::MethodNotAllowed.into_response(),
        },
    }
}

// ── Catalog ───────────────────────────────────────────────────────────────────

async fn handle_catalog(state: Arc<AppState>) -> Response {
    let repos = state.store.list_repositories().await;
    Json(CatalogResponse { repositories: repos }).into_response()
}

// ── Tags list ─────────────────────────────────────────────────────────────────

async fn handle_tags_list(state: Arc<AppState>, name: String) -> Response {
    let mut tags = state.store.list_tags(&name).await;
    tags.sort();
    Json(TagsListResponse { name, tags }).into_response()
}

// ── Manifest handlers ─────────────────────────────────────────────────────────

async fn handle_get_manifest(
    state: Arc<AppState>,
    name: String,
    reference: String,
) -> Response {
    match state.store.get_manifest(&name, &reference).await {
        None => RegistryError::ManifestUnknown.into_response(),
        Some(m) => {
            let digest = m.digest.clone();
            let mt = m.media_type.clone();
            (
                StatusCode::OK,
                [
                    ("Content-Type", mt),
                    ("Docker-Content-Digest", digest),
                    ("Docker-Distribution-Api-Version", "registry/2.0".to_string()),
                ],
                m.content,
            )
                .into_response()
        }
    }
}

async fn handle_head_manifest(
    state: Arc<AppState>,
    name: String,
    reference: String,
) -> Response {
    match state.store.get_manifest(&name, &reference).await {
        None => RegistryError::ManifestUnknown.into_response(),
        Some(m) => {
            let size = m.content.len().to_string();
            (
                StatusCode::OK,
                [
                    ("Content-Type", m.media_type),
                    ("Content-Length", size),
                    ("Docker-Content-Digest", m.digest),
                    ("Docker-Distribution-Api-Version", "registry/2.0".to_string()),
                ],
                "",
            )
                .into_response()
        }
    }
}

async fn handle_put_manifest(
    state: Arc<AppState>,
    name: String,
    reference: String,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let media_type = extract_content_type(&headers);

    if !is_manifest_media_type(&media_type) {
        return RegistryError::InvalidManifest(format!("unsupported media type: {media_type}"))
            .into_response();
    }

    // Validate JSON
    if serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return RegistryError::InvalidManifest("body is not valid JSON".to_string())
            .into_response();
    }

    // Tag immutability check
    if state.policy.is_tag_immutable(&name, &reference).await {
        return RegistryError::PolicyViolation(format!(
            "tag '{reference}' in repository '{name}' is immutable"
        ))
        .into_response();
    }

    let content = body.to_vec();
    match state
        .store
        .put_manifest(&name, &reference, media_type.clone(), content.clone())
        .await
    {
        Err(e) => RegistryError::Storage(e).into_response(),
        Ok(digest) => {
            let location = format!("/v2/{name}/manifests/{digest}");
            // Fire push webhook (non-blocking).
            let tag = if reference.starts_with("sha256:") {
                None
            } else {
                Some(reference.as_str())
            };
            state
                .webhooks
                .fire(
                    crate::types::WebhookEvent::Push,
                    &name,
                    Some(&digest),
                    tag,
                )
                .await;
            // Trigger replication (non-blocking).
            state
                .replication
                .replicate_manifest(&name, &reference, content.clone(), media_type, digest.clone())
                .await;
            // Trigger scan (non-blocking; runs inline but short-circuits quickly for noop).
            state.scanner.trigger(&digest, content).await;

            (
                StatusCode::CREATED,
                [
                    ("Location", location),
                    ("Docker-Content-Digest", digest),
                    ("Docker-Distribution-Api-Version", "registry/2.0".to_string()),
                ],
                "",
            )
                .into_response()
        }
    }
}

async fn handle_delete_manifest(
    state: Arc<AppState>,
    name: String,
    reference: String,
) -> Response {
    if state.store.delete_manifest(&name, &reference).await {
        state
            .webhooks
            .fire(crate::types::WebhookEvent::Delete, &name, Some(&reference), None)
            .await;
        StatusCode::ACCEPTED.into_response()
    } else {
        RegistryError::ManifestUnknown.into_response()
    }
}

// ── Blob handlers ─────────────────────────────────────────────────────────────

async fn handle_get_blob(
    state: Arc<AppState>,
    _name: String,
    digest: String,
) -> Response {
    match state.store.get_blob(&digest).await {
        None => RegistryError::BlobUnknown.into_response(),
        Some(blob) => {
            state
                .webhooks
                .fire(
                    crate::types::WebhookEvent::Pull,
                    &_name,
                    Some(&digest),
                    None,
                )
                .await;
            (
                StatusCode::OK,
                [
                    ("Content-Type", "application/octet-stream".to_string()),
                    ("Content-Length", blob.size.to_string()),
                    ("Docker-Content-Digest", blob.digest.clone()),
                ],
                blob.content,
            )
                .into_response()
        }
    }
}

async fn handle_head_blob(
    state: Arc<AppState>,
    _name: String,
    digest: String,
) -> Response {
    match state.store.get_blob(&digest).await {
        None => RegistryError::BlobUnknown.into_response(),
        Some(blob) => (
            StatusCode::OK,
            [
                ("Content-Length", blob.size.to_string()),
                ("Docker-Content-Digest", blob.digest),
            ],
            "",
        )
            .into_response(),
    }
}

async fn handle_delete_blob(
    state: Arc<AppState>,
    _name: String,
    digest: String,
) -> Response {
    if state.store.delete_blob(&digest).await {
        StatusCode::ACCEPTED.into_response()
    } else {
        RegistryError::BlobUnknown.into_response()
    }
}

// ── Upload handlers ───────────────────────────────────────────────────────────

/// POST /v2/:name/blobs/uploads/
/// Supports monolithic upload (body + digest query) or initiates a session.
async fn handle_start_upload(
    state: Arc<AppState>,
    name: String,
    digest_q: DigestQuery,
    body: Bytes,
) -> Response {
    // Monolithic upload: body + ?digest=sha256:...
    if let Some(ref expected) = digest_q.digest {
        if !body.is_empty() {
            return match state.store.put_blob(body.to_vec(), Some(expected)).await {
                Ok(digest) => {
                    let location = format!("/v2/{name}/blobs/{digest}");
                    (
                        StatusCode::CREATED,
                        [
                            ("Location", location),
                            ("Docker-Content-Digest", digest),
                        ],
                        "",
                    )
                        .into_response()
                }
                Err(_) => RegistryError::DigestMismatch {
                    expected: expected.clone(),
                    got: compute_digest(&body),
                }
                .into_response(),
            };
        }
    }

    // Chunked session
    let session_id = state.store.create_session(&name).await;
    let location = format!("/v2/{name}/blobs/uploads/{session_id}");
    (
        StatusCode::ACCEPTED,
        [
            ("Location", location),
            ("Docker-Upload-UUID", session_id),
            ("Range", "0-0".to_string()),
        ],
        "",
    )
        .into_response()
}

/// PATCH /v2/:name/blobs/uploads/:session_id
async fn handle_patch_upload(
    state: Arc<AppState>,
    name: String,
    session_id: String,
    body: Bytes,
) -> Response {
    match state.store.append_session(&session_id, body.to_vec()).await {
        None => RegistryError::UploadNotFound(session_id).into_response(),
        Some(offset) => {
            let location = format!("/v2/{name}/blobs/uploads/{session_id}");
            let range = format!("0-{}", offset.saturating_sub(1));
            (
                StatusCode::ACCEPTED,
                [
                    ("Location", location),
                    ("Range", range),
                    ("Docker-Upload-UUID", session_id),
                ],
                "",
            )
                .into_response()
        }
    }
}

/// PUT /v2/:name/blobs/uploads/:session_id?digest=sha256:...
async fn handle_complete_upload(
    state: Arc<AppState>,
    name: String,
    session_id: String,
    digest_q: DigestQuery,
    body: Bytes,
) -> Response {
    // Append any trailing body chunk before finalising.
    if !body.is_empty() {
        if state.store.append_session(&session_id, body.to_vec()).await.is_none() {
            return RegistryError::UploadNotFound(session_id).into_response();
        }
    }

    let expected = match digest_q.digest {
        Some(d) => d,
        None => {
            return RegistryError::DigestMismatch {
                expected: "sha256:<required>".to_string(),
                got: String::new(),
            }
            .into_response()
        }
    };

    match state.store.complete_session(&session_id, &expected).await {
        Ok((_repo, digest)) => {
            let location = format!("/v2/{name}/blobs/{digest}");
            (
                StatusCode::CREATED,
                [
                    ("Location", location),
                    ("Docker-Content-Digest", digest),
                ],
                "",
            )
                .into_response()
        }
        Err(e) => RegistryError::DigestMismatch {
            expected,
            got: e,
        }
        .into_response(),
    }
}

/// DELETE /v2/:name/blobs/uploads/:session_id  — cancel an in-progress upload.
async fn handle_cancel_upload(
    state: Arc<AppState>,
    _name: String,
    session_id: String,
) -> Response {
    if state.store.delete_session(&session_id).await {
        StatusCode::NO_CONTENT.into_response()
    } else {
        RegistryError::UploadNotFound(session_id).into_response()
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        policy::PolicyManager,
        replication::ReplicationManager,
        scan::ScanManager,
        store::RegistryStore,
        webhook::WebhookManager,
        AppState,
    };
    use axum::{body::to_bytes, http::Request};
    use tower::util::ServiceExt;

    fn make_state() -> Arc<AppState> {
        let store = Arc::new(RegistryStore::new());
        Arc::new(AppState {
            store: Arc::clone(&store),
            webhooks: Arc::new(WebhookManager::new(Arc::clone(&store))),
            replication: Arc::new(ReplicationManager::new(Arc::clone(&store))),
            scanner: Arc::new(ScanManager::new(Arc::clone(&store))),
            policy: Arc::new(PolicyManager::new(Arc::clone(&store))),
        })
    }

    fn app(state: Arc<AppState>) -> axum::Router {
        crate::routes::create_router(state)
    }

    // ── parse_path ────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_catalog() {
        assert!(matches!(parse_path("_catalog"), Some(V2Action::Catalog)));
    }

    #[test]
    fn test_parse_manifest_by_tag() {
        match parse_path("library/nginx/manifests/latest") {
            Some(V2Action::Manifest { name, reference }) => {
                assert_eq!(name, "library/nginx");
                assert_eq!(reference, "latest");
            }
            other => panic!("expected Manifest, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_manifest_by_digest() {
        match parse_path("myrepo/manifests/sha256:abc123") {
            Some(V2Action::Manifest { name, reference }) => {
                assert_eq!(name, "myrepo");
                assert_eq!(reference, "sha256:abc123");
            }
            other => panic!("expected Manifest, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_tags_list() {
        match parse_path("myrepo/tags/list") {
            Some(V2Action::TagsList { name }) => assert_eq!(name, "myrepo"),
            other => panic!("expected TagsList, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_upload_start() {
        match parse_path("myrepo/blobs/uploads/") {
            Some(V2Action::UploadStart { name }) => assert_eq!(name, "myrepo"),
            other => panic!("expected UploadStart, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_upload_chunk() {
        match parse_path("myrepo/blobs/uploads/session-uuid-123") {
            Some(V2Action::UploadChunk { name, session_id }) => {
                assert_eq!(name, "myrepo");
                assert_eq!(session_id, "session-uuid-123");
            }
            other => panic!("expected UploadChunk, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_blob_get() {
        match parse_path("myrepo/blobs/sha256:deadbeef") {
            Some(V2Action::BlobGet { name, digest }) => {
                assert_eq!(name, "myrepo");
                assert_eq!(digest, "sha256:deadbeef");
            }
            other => panic!("expected BlobGet, got {other:?}"),
        }
    }

    // ── HTTP integration tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_v2_check_returns_200() {
        let state = make_state();
        let router = app(state);
        let resp = router
            .oneshot(Request::builder().uri("/v2/").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_get_manifest_not_found() {
        let state = make_state();
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .uri("/v2/missing/manifests/latest")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_put_and_get_manifest() {
        let state = make_state();
        let manifest = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": MEDIA_MANIFEST_V2, "size": 0, "digest": "sha256:empty" },
            "layers": []
        });
        let body = serde_json::to_vec(&manifest).unwrap();

        let put_resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/myrepo/manifests/latest")
                    .header("Content-Type", MEDIA_MANIFEST_V2)
                    .body(axum::body::Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put_resp.status(), StatusCode::CREATED);
        let digest = put_resp.headers()["docker-content-digest"].to_str().unwrap().to_string();

        let get_resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri("/v2/myrepo/manifests/latest")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);
        assert_eq!(
            get_resp.headers()["docker-content-digest"].to_str().unwrap(),
            digest
        );
    }

    #[tokio::test]
    async fn test_head_manifest() {
        let state = make_state();
        let body = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2, "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": MEDIA_MANIFEST_V2, "size": 0, "digest": "sha256:x" },
            "layers": []
        }))
        .unwrap();
        app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/repo/manifests/v1")
                    .header("Content-Type", MEDIA_MANIFEST_V2)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri("/v2/repo/manifests/v1")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key("docker-content-digest"));
    }

    #[tokio::test]
    async fn test_delete_manifest() {
        let state = make_state();
        let body = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2, "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": MEDIA_MANIFEST_V2, "size": 0, "digest": "sha256:x" },
            "layers": []
        }))
        .unwrap();
        app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/repo/manifests/todelete")
                    .header("Content-Type", MEDIA_MANIFEST_V2)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let del = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/v2/repo/manifests/todelete")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::ACCEPTED);

        let get = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri("/v2/repo/manifests/todelete")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_blob_upload_chunked_and_download() {
        let state = make_state();
        let data = b"hello world blob data";
        let expected_digest = compute_digest(data);

        // Start upload
        let start = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/myrepo/blobs/uploads/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::ACCEPTED);
        let location = start.headers()["location"].to_str().unwrap().to_string();
        let session_id = location.rsplit('/').next().unwrap().to_string();

        // PATCH chunk
        let patch = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(&format!("/v2/myrepo/blobs/uploads/{session_id}"))
                    .body(axum::body::Body::from(data.as_ref()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(patch.status(), StatusCode::ACCEPTED);

        // PUT finalize
        let put = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&format!("/v2/myrepo/blobs/uploads/{session_id}?digest={expected_digest}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::CREATED);

        // GET blob
        let get = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri(&format!("/v2/myrepo/blobs/{expected_digest}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), StatusCode::OK);
        let body = to_bytes(get.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), data);
    }

    #[tokio::test]
    async fn test_blob_upload_wrong_digest_rejected() {
        let state = make_state();

        let start = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/repo/blobs/uploads/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let location = start.headers()["location"].to_str().unwrap().to_string();
        let session_id = location.rsplit('/').next().unwrap().to_string();

        app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(&format!("/v2/repo/blobs/uploads/{session_id}"))
                    .body(axum::body::Body::from("real data"))
                    .unwrap(),
            )
            .await
            .unwrap();

        let put = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(&format!("/v2/repo/blobs/uploads/{session_id}?digest=sha256:wrongwrongwrong"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(put.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_monolithic_blob_upload() {
        let state = make_state();
        let data = b"monolithic blob";
        let digest = compute_digest(data);

        let resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&format!("/v2/repo/blobs/uploads/?digest={digest}"))
                    .body(axum::body::Body::from(data.as_ref()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(
            resp.headers()["docker-content-digest"].to_str().unwrap(),
            digest
        );
    }

    #[tokio::test]
    async fn test_head_blob() {
        let state = make_state();
        let data = b"some blob";
        let digest = state.store.put_blob(data.to_vec(), None).await.unwrap();

        let resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri(&format!("/v2/myrepo/blobs/{digest}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()["docker-content-digest"].to_str().unwrap(),
            digest
        );
    }

    #[tokio::test]
    async fn test_catalog_lists_repos() {
        let state = make_state();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2, "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": MEDIA_MANIFEST_V2, "size": 0, "digest": "sha256:x" },
            "layers": []
        }))
        .unwrap();
        for repo in ["alpha", "beta", "gamma"] {
            app(Arc::clone(&state))
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(&format!("/v2/{repo}/manifests/latest"))
                        .header("Content-Type", MEDIA_MANIFEST_V2)
                        .body(axum::body::Body::from(manifest.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
        }
        let resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri("/v2/_catalog")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cat: CatalogResponse = serde_json::from_slice(&body).unwrap();
        assert!(cat.repositories.contains(&"alpha".to_string()));
        assert!(cat.repositories.contains(&"beta".to_string()));
        assert!(cat.repositories.contains(&"gamma".to_string()));
    }

    #[tokio::test]
    async fn test_tags_list() {
        let state = make_state();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2, "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": MEDIA_MANIFEST_V2, "size": 0, "digest": "sha256:x" },
            "layers": []
        }))
        .unwrap();
        for tag in ["v1.0", "v2.0", "latest"] {
            app(Arc::clone(&state))
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(&format!("/v2/myapp/manifests/{tag}"))
                        .header("Content-Type", MEDIA_MANIFEST_V2)
                        .body(axum::body::Body::from(manifest.clone()))
                        .unwrap(),
                )
                .await
                .unwrap();
        }
        let resp = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .uri("/v2/myapp/tags/list")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tl: TagsListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(tl.name, "myapp");
        assert!(tl.tags.contains(&"latest".to_string()));
        assert!(tl.tags.contains(&"v1.0".to_string()));
    }

    #[tokio::test]
    async fn test_digest_verification_on_manifest_push() {
        let state = make_state();
        let body = b"not valid json at all!!!";
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/repo/manifests/bad")
                    .header("Content-Type", MEDIA_MANIFEST_V2)
                    .body(axum::body::Body::from(body.as_ref()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_immutable_tag_policy_blocks_push() {
        use crate::types::TagPolicy;
        let state = make_state();
        state
            .policy
            .set_tag_policy(
                "locked",
                TagPolicy { immutable_tags: vec!["v1.0.0".to_string()], all_immutable: false },
            )
            .await;

        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2, "mediaType": MEDIA_MANIFEST_V2,
            "config": { "mediaType": MEDIA_MANIFEST_V2, "size": 0, "digest": "sha256:x" },
            "layers": []
        }))
        .unwrap();

        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/locked/manifests/v1.0.0")
                    .header("Content-Type", MEDIA_MANIFEST_V2)
                    .body(axum::body::Body::from(manifest))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn test_oci_manifest_accepted() {
        let state = make_state();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": MEDIA_OCI_MANIFEST,
            "config": { "mediaType": "application/vnd.oci.image.config.v1+json", "size": 0, "digest": "sha256:x" },
            "layers": []
        }))
        .unwrap();
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/oci/manifests/latest")
                    .header("Content-Type", MEDIA_OCI_MANIFEST)
                    .body(axum::body::Body::from(manifest))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_manifest_list_accepted() {
        let state = make_state();
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 2,
            "mediaType": MEDIA_MANIFEST_LIST,
            "manifests": [{
                "mediaType": MEDIA_MANIFEST_V2,
                "size": 100,
                "digest": "sha256:abc",
                "platform": { "os": "linux", "architecture": "amd64" }
            }]
        }))
        .unwrap();
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/v2/multiarch/manifests/latest")
                    .header("Content-Type", MEDIA_MANIFEST_LIST)
                    .body(axum::body::Body::from(manifest))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_get_blob_not_found() {
        let state = make_state();
        let resp = app(state)
            .oneshot(
                Request::builder()
                    .uri("/v2/myrepo/blobs/sha256:nonexistent")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_cancel_upload_session() {
        let state = make_state();
        let start = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v2/repo/blobs/uploads/")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let location = start.headers()["location"].to_str().unwrap().to_string();
        let session_id = location.rsplit('/').next().unwrap().to_string();

        let del = app(Arc::clone(&state))
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(&format!("/v2/repo/blobs/uploads/{session_id}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del.status(), StatusCode::NO_CONTENT);
    }
}
