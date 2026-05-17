// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/server/registry/handler.go (Docker Registry V2 dispatch)
//! Docker Registry HTTP API V2 compatibility layer.
//!
//! Implements the Docker Distribution Spec v2 so that `docker pull`,
//! `docker push`, and any OCI-compatible tool can use cave-registry
//! as a drop-in container registry.
//!
//! ## Endpoints
//! - GET  /v2/                               — API version check
//! - GET  /v2/_catalog                       — list repositories
//! - GET  /v2/{name}/manifests/{reference}     — pull manifest
//! - HEAD /v2/{name}/manifests/{reference}     — manifest existence check
//! - PUT  /v2/{name}/manifests/{reference}     — push manifest
//! - GET  /v2/{name}/blobs/{digest}            — pull blob
//! - HEAD /v2/{name}/blobs/{digest}            — blob existence check
//! - POST /v2/{name}/blobs/uploads/           — initiate blob upload
//! - PATCH /v2/{name}/blobs/uploads/{uuid}     — chunked blob upload
//! - PUT  /v2/{name}/blobs/uploads/{uuid}      — complete blob upload
//!
//! ## Required headers (Docker spec)
//! - Docker-Distribution-API-Version: registry/2.0   (all responses)
//! - Docker-Content-Digest: sha256:<hex>              (manifest responses)
//!
//! ## Reference
//! - https://github.com/opencontainers/distribution-spec/blob/main/spec.md

use crate::harbor::State as RegistryState;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header::HeaderName, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Static header values required by the Docker Distribution spec.
const REGISTRY_VERSION: &str = "registry/2.0";
const HEADER_DISTRIBUTION_API_VERSION: &str = "docker-distribution-api-version";
const HEADER_CONTENT_DIGEST: &str = "docker-content-digest";

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// GET /v2/_catalog response.
#[derive(Debug, Serialize)]
pub struct CatalogResponse {
    pub repositories: Vec<String>,
}

/// GET /v2/{name}/tags/list response.
#[derive(Debug, Serialize)]
pub struct TagsResponse {
    pub name: String,
    pub tags: Vec<String>,
}

/// OCI / Docker manifest (simplified — real impl deserialises full spec).
#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u8,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: ManifestDescriptor,
    pub layers: Vec<ManifestDescriptor>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub digest: String,
}

/// Query params for catalog listing.
#[derive(Debug, Deserialize)]
pub struct CatalogParams {
    pub n: Option<u32>,
    pub last: Option<String>,
}

// ---------------------------------------------------------------------------
// Helper — build the standard distribution API version header
// ---------------------------------------------------------------------------

fn dist_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(HEADER_DISTRIBUTION_API_VERSION),
        HeaderValue::from_static(REGISTRY_VERSION),
    );
    headers
}

fn dist_headers_with_digest(digest: &str) -> HeaderMap {
    let mut headers = dist_headers();
    if let Ok(v) = HeaderValue::from_str(digest) {
        headers.insert(HeaderName::from_static(HEADER_CONTENT_DIGEST), v);
    }
    headers
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn docker_v2_router(state: Arc<RegistryState>) -> Router {
    Router::new()
        // Version check
        .route("/v2/", get(version_check))
        // Catalog
        .route("/v2/_catalog", get(catalog))
        // Manifests — note: :name is a single path segment here.
        // Multi-segment names (org/image) require middleware-level rewriting
        // or a wildcard route; see CAVE-REGISTRY-MULTI-SEGMENT-TODO.
        .route("/v2/{name}/manifests/{reference}", get(pull_manifest)
            .head(head_manifest)
            .put(push_manifest))
        // Blobs
        .route("/v2/{name}/blobs/{digest}", get(pull_blob).head(head_blob))
        // Blob upload (chunked)
        .route("/v2/{name}/blobs/uploads/", post(initiate_upload))
        .route("/v2/{name}/blobs/uploads/{upload_uuid}",
            patch(upload_chunk).put(complete_upload))
        // Tag list (bonus — used by many tools)
        .route("/v2/{name}/tags/list", get(list_tags))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// GET /v2/ — version check
// ---------------------------------------------------------------------------

/// Returns 200 with the Docker-Distribution-API-Version header.
/// Docker clients MUST receive this before attempting any other operation.
async fn version_check() -> Response {
    (StatusCode::OK, dist_headers(), "{}").into_response()
}

// ---------------------------------------------------------------------------
// GET /v2/_catalog — list repositories
// ---------------------------------------------------------------------------

async fn catalog(
    State(_state): State<Arc<RegistryState>>,
    Query(_params): Query<CatalogParams>,
) -> Response {
    // TODO: query DB for repository list
    let body = Json(CatalogResponse { repositories: vec![] });
    (StatusCode::OK, dist_headers(), body).into_response()
}

// ---------------------------------------------------------------------------
// GET /v2/{name}/manifests/{reference} — pull manifest
// ---------------------------------------------------------------------------

async fn pull_manifest(
    State(_state): State<Arc<RegistryState>>,
    Path((name, reference)): Path<(String, String)>,
) -> Response {
    tracing::debug!(name = %name, reference = %reference, "pull_manifest");
    // TODO: load manifest from store, compute sha256 digest
    let digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let headers = dist_headers_with_digest(digest);
    // 404 until real storage is implemented
    (StatusCode::NOT_FOUND, headers,
        Json(serde_json::json!({
            "errors": [{"code": "MANIFEST_UNKNOWN", "message": "manifest unknown"}]
        }))).into_response()
}

// ---------------------------------------------------------------------------
// HEAD /v2/{name}/manifests/{reference} — manifest existence check
// ---------------------------------------------------------------------------

async fn head_manifest(
    State(_state): State<Arc<RegistryState>>,
    Path((name, reference)): Path<(String, String)>,
) -> Response {
    tracing::debug!(name = %name, reference = %reference, "head_manifest");
    // TODO: check manifest exists in store
    (StatusCode::NOT_FOUND, dist_headers()).into_response()
}

// ---------------------------------------------------------------------------
// PUT /v2/{name}/manifests/{reference} — push manifest
// ---------------------------------------------------------------------------

async fn push_manifest(
    State(_state): State<Arc<RegistryState>>,
    Path((name, reference)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    tracing::debug!(name = %name, reference = %reference, bytes = body.len(), "push_manifest");
    // TODO: persist manifest, compute digest
    let digest = format!(
        "sha256:{:064x}",
        body.len() as u64 // placeholder — replace with real sha256
    );
    let mut headers = dist_headers_with_digest(&digest);
    if let Ok(location) = HeaderValue::from_str(&format!("/v2/{name}/manifests/{reference}")) {
        headers.insert(axum::http::header::LOCATION, location);
    }
    (StatusCode::CREATED, headers).into_response()
}

// ---------------------------------------------------------------------------
// GET /v2/{name}/blobs/{digest} — pull blob
// ---------------------------------------------------------------------------

async fn pull_blob(
    State(_state): State<Arc<RegistryState>>,
    Path((name, digest)): Path<(String, String)>,
) -> Response {
    tracing::debug!(name = %name, digest = %digest, "pull_blob");
    // TODO: stream blob from object store
    let headers = dist_headers_with_digest(&digest);
    (StatusCode::NOT_FOUND, headers,
        Json(serde_json::json!({
            "errors": [{"code": "BLOB_UNKNOWN", "message": "blob unknown to registry"}]
        }))).into_response()
}

// ---------------------------------------------------------------------------
// HEAD /v2/{name}/blobs/{digest} — blob existence check
// ---------------------------------------------------------------------------

async fn head_blob(
    State(_state): State<Arc<RegistryState>>,
    Path((name, digest)): Path<(String, String)>,
) -> Response {
    tracing::debug!(name = %name, digest = %digest, "head_blob");
    // TODO: check blob exists in object store
    (StatusCode::NOT_FOUND, dist_headers()).into_response()
}

// ---------------------------------------------------------------------------
// POST /v2/{name}/blobs/uploads/ — initiate blob upload
// ---------------------------------------------------------------------------

async fn initiate_upload(
    State(_state): State<Arc<RegistryState>>,
    Path(name): Path<String>,
) -> Response {
    tracing::debug!(name = %name, "initiate_upload");
    // TODO: create upload session, return UUID
    let upload_uuid = uuid::Uuid::new_v4().to_string();
    let location = format!("/v2/{name}/blobs/uploads/{upload_uuid}");
    let mut headers = dist_headers();
    if let Ok(v) = HeaderValue::from_str(&location) {
        headers.insert(axum::http::header::LOCATION, v);
    }
    // Range: 0-0 indicates 0 bytes uploaded so far
    headers.insert(
        axum::http::header::RANGE,
        HeaderValue::from_static("0-0"),
    );
    (StatusCode::ACCEPTED, headers).into_response()
}

// ---------------------------------------------------------------------------
// PATCH /v2/{name}/blobs/uploads/{uuid} — chunked blob upload
// ---------------------------------------------------------------------------

async fn upload_chunk(
    State(_state): State<Arc<RegistryState>>,
    Path((name, upload_uuid)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    tracing::debug!(name = %name, uuid = %upload_uuid, bytes = body.len(), "upload_chunk");
    // TODO: append chunk to upload session
    let location = format!("/v2/{name}/blobs/uploads/{upload_uuid}");
    let range = format!("0-{}", body.len().saturating_sub(1));
    let mut headers = dist_headers();
    if let Ok(v) = HeaderValue::from_str(&location) {
        headers.insert(axum::http::header::LOCATION, v);
    }
    if let Ok(v) = HeaderValue::from_str(&range) {
        headers.insert(axum::http::header::RANGE, v);
    }
    (StatusCode::ACCEPTED, headers).into_response()
}

// ---------------------------------------------------------------------------
// PUT /v2/{name}/blobs/uploads/{uuid} — complete blob upload
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CompleteUploadParams {
    pub digest: String,
}

async fn complete_upload(
    State(_state): State<Arc<RegistryState>>,
    Path((name, upload_uuid)): Path<(String, String)>,
    Query(params): Query<CompleteUploadParams>,
    body: Bytes,
) -> Response {
    tracing::debug!(
        name   = %name,
        uuid   = %upload_uuid,
        digest = %params.digest,
        bytes  = body.len(),
        "complete_upload"
    );
    // TODO: finalise upload, verify digest, persist blob
    let location = format!("/v2/{name}/blobs/{}", params.digest);
    let mut headers = dist_headers_with_digest(&params.digest);
    if let Ok(v) = HeaderValue::from_str(&location) {
        headers.insert(axum::http::header::LOCATION, v);
    }
    (StatusCode::CREATED, headers).into_response()
}

// ---------------------------------------------------------------------------
// GET /v2/{name}/tags/list
// ---------------------------------------------------------------------------

async fn list_tags(
    State(_state): State<Arc<RegistryState>>,
    Path(name): Path<String>,
) -> Response {
    // TODO: query tags from store
    let body = Json(TagsResponse { name: name.clone(), tags: vec![] });
    (StatusCode::OK, dist_headers(), body).into_response()
}
