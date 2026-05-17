// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: goharbor/harbor@c80058d52f555c9bd4552ea14c9d3e73ba0e4b12 src/server/registry/handler.go + OCI distribution-spec v1.1
//! Docker Registry V2 + OCI Distribution Spec 1.1 routes.
//!
//! All repository-scoped endpoints are dispatched through a single
//! catch-all handler because axum cannot place wildcards in the middle
//! of a path (e.g. `/v2/{*name}/manifests/{ref}` is not legal).

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, Method, StatusCode},
    response::Response,
    routing::{any, get},
    Router,
};
use std::{collections::HashMap, sync::Arc};

use crate::harbor::{
    models::{
        classify_v2_path, Catalog, ReferrersResponse, RegistryErrors, TagList, V2Op,
        ERR_BLOB_UNKNOWN, ERR_BLOB_UPLOAD_INVALID, ERR_BLOB_UPLOAD_UNKNOWN,
        ERR_DIGEST_INVALID, ERR_MANIFEST_UNKNOWN, ERR_NAME_UNKNOWN, ERR_UNSUPPORTED,
    },
    storage::RegistryStorage,
    RegistryState,
};

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: Arc<RegistryState>) -> Router {
    Router::new()
        .route("/v2/", get(version_check))
        .route("/v2/_catalog", get(catalog))
        .route("/v2/{*path}", any(dispatch))
        .with_state(state)
}

// ── Response helpers ──────────────────────────────────────────────────────────

fn json_response(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body))
        .unwrap()
}

fn registry_error(status: StatusCode, code: &str, message: &str) -> Response {
    json_response(status, RegistryErrors::new(code, message).to_json())
}

fn method_not_allowed() -> Response {
    registry_error(StatusCode::METHOD_NOT_ALLOWED, ERR_UNSUPPORTED, "method not allowed")
}

// ── Version check ─────────────────────────────────────────────────────────────

/// GET /v2/  — Docker Distribution API version check.
async fn version_check() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header("Docker-Distribution-API-Version", "registry/2.0")
        .header(header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from("{}"))
        .unwrap()
}

// ── Catalog ───────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct PaginationQuery {
    n: Option<usize>,
    last: Option<String>,
}

/// GET /v2/_catalog  — list repositories.
async fn catalog(
    State(state): State<Arc<RegistryState>>,
    Query(q): Query<PaginationQuery>,
) -> Response {
    let mut repos = state.storage.list_repos().await;

    if let Some(ref last) = q.last {
        repos = repos
            .into_iter()
            .skip_while(|r| r != last)
            .skip(1)
            .collect();
    }
    if let Some(n) = q.n {
        repos.truncate(n);
    }

    let cat = Catalog { repositories: repos };
    json_response(StatusCode::OK, serde_json::to_string(&cat).unwrap())
}

// ── Dispatcher ────────────────────────────────────────────────────────────────

async fn dispatch(
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    State(state): State<Arc<RegistryState>>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    match classify_v2_path(&path) {
        V2Op::TagsList(name) => tags_list(state, &name, &query).await,

        V2Op::Manifest(name, reference) => match method {
            Method::GET => get_manifest(state, &name, &reference, false).await,
            Method::HEAD => get_manifest(state, &name, &reference, true).await,
            Method::PUT => put_manifest(state, &name, &reference, headers, body).await,
            Method::DELETE => delete_manifest(state, &name, &reference).await,
            _ => method_not_allowed(),
        },

        V2Op::Blob(name, digest) => match method {
            Method::GET => get_blob(state, &name, &digest, false).await,
            Method::HEAD => get_blob(state, &name, &digest, true).await,
            Method::DELETE => delete_blob(state, &name, &digest).await,
            _ => method_not_allowed(),
        },

        V2Op::BlobUploadInitiate(name) if method == Method::POST => {
            initiate_upload(state, &name, &query).await
        }

        V2Op::BlobUploadSession(name, uuid) => match method {
            Method::PATCH => patch_upload(state, &name, &uuid, headers, body).await,
            Method::PUT => complete_upload(state, &name, &uuid, &query, body).await,
            Method::DELETE => cancel_upload(state, &name, &uuid).await,
            _ => method_not_allowed(),
        },

        V2Op::Referrers(name, digest) if method == Method::GET => {
            get_referrers(state, &name, &digest, &query).await
        }

        _ => registry_error(StatusCode::NOT_FOUND, ERR_NAME_UNKNOWN, "resource not found"),
    }
}

// ── Tags ──────────────────────────────────────────────────────────────────────

async fn tags_list(
    state: Arc<RegistryState>,
    name: &str,
    query: &HashMap<String, String>,
) -> Response {
    let mut tags = state.storage.list_tags(name).await;

    if let Some(last) = query.get("last") {
        tags = tags.into_iter().skip_while(|t| t != last).skip(1).collect();
    }
    if let Some(n) = query.get("n").and_then(|v| v.parse::<usize>().ok()) {
        tags.truncate(n);
    }

    let list = TagList {
        name: name.to_string(),
        tags,
    };
    json_response(StatusCode::OK, serde_json::to_string(&list).unwrap())
}

// ── Manifests ────────────────────────────────────────────────────────────────

async fn get_manifest(
    state: Arc<RegistryState>,
    name: &str,
    reference: &str,
    head_only: bool,
) -> Response {
    match state.storage.get_manifest(name, reference).await {
        None => registry_error(StatusCode::NOT_FOUND, ERR_MANIFEST_UNKNOWN, "manifest unknown"),
        Some(entry) => {
            let size = entry.data.len();
            let builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, entry.content_type.clone())
                .header("Docker-Content-Digest", entry.digest.clone())
                .header(header::CONTENT_LENGTH, size)
                .header("Docker-Distribution-API-Version", "registry/2.0");

            if head_only {
                builder.body(axum::body::Body::empty()).unwrap()
            } else {
                builder.body(axum::body::Body::from(entry.data)).unwrap()
            }
        }
    }
}

async fn put_manifest(
    state: Arc<RegistryState>,
    name: &str,
    reference: &str,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_string();

    // Extract subject_digest and artifact_type from the manifest for OCI 1.1 referrers
    let (subject_digest, artifact_type) = extract_oci_fields(&body);

    let digest = state
        .storage
        .store_manifest(name, reference, content_type, body, subject_digest, artifact_type)
        .await;

    Response::builder()
        .status(StatusCode::CREATED)
        .header(header::LOCATION, format!("/v2/{}/manifests/{}", name, reference))
        .header("Docker-Content-Digest", &digest)
        .header(header::CONTENT_LENGTH, 0)
        .body(axum::body::Body::empty())
        .unwrap()
}

async fn delete_manifest(
    state: Arc<RegistryState>,
    name: &str,
    reference: &str,
) -> Response {
    if state.storage.delete_manifest(name, reference).await {
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(axum::body::Body::empty())
            .unwrap()
    } else {
        registry_error(StatusCode::NOT_FOUND, ERR_MANIFEST_UNKNOWN, "manifest unknown")
    }
}

/// Pull subject + artifactType from a raw OCI manifest JSON (best-effort).
fn extract_oci_fields(data: &[u8]) -> (Option<String>, Option<String>) {
    let v: serde_json::Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return (None, None),
    };
    let subject = v
        .get("subject")
        .and_then(|s| s.get("digest"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string());
    let artifact_type = v
        .get("artifactType")
        .and_then(|a| a.as_str())
        .map(|s| s.to_string());
    (subject, artifact_type)
}

// ── Blobs ─────────────────────────────────────────────────────────────────────

async fn get_blob(
    state: Arc<RegistryState>,
    _name: &str,
    digest: &str,
    head_only: bool,
) -> Response {
    match state.storage.get_blob(digest).await {
        None => registry_error(StatusCode::NOT_FOUND, ERR_BLOB_UNKNOWN, "blob unknown"),
        Some(data) => {
            let size = data.len();
            let builder = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .header(header::CONTENT_LENGTH, size)
                .header("Docker-Content-Digest", digest);

            if head_only {
                builder.body(axum::body::Body::empty()).unwrap()
            } else {
                builder.body(axum::body::Body::from(data)).unwrap()
            }
        }
    }
}

async fn delete_blob(
    state: Arc<RegistryState>,
    name: &str,
    digest: &str,
) -> Response {
    if state.storage.delete_blob(digest, name).await {
        Response::builder()
            .status(StatusCode::ACCEPTED)
            .body(axum::body::Body::empty())
            .unwrap()
    } else {
        registry_error(StatusCode::NOT_FOUND, ERR_BLOB_UNKNOWN, "blob unknown")
    }
}

// ── Blob uploads ──────────────────────────────────────────────────────────────

async fn initiate_upload(
    state: Arc<RegistryState>,
    name: &str,
    query: &HashMap<String, String>,
) -> Response {
    // Cross-repo mount
    if let (Some(mount), Some(from)) = (query.get("mount"), query.get("from")) {
        if state.storage.mount_blob(mount, from, name).await {
            return Response::builder()
                .status(StatusCode::CREATED)
                .header(
                    header::LOCATION,
                    format!("/v2/{}/blobs/{}", name, mount),
                )
                .header("Docker-Content-Digest", mount.as_str())
                .header(header::CONTENT_LENGTH, 0)
                .body(axum::body::Body::empty())
                .unwrap();
        }
        // Mount failed — fall through to normal upload
    }

    let uuid = state.storage.start_upload(name).await;
    Response::builder()
        .status(StatusCode::ACCEPTED)
        .header(
            header::LOCATION,
            format!("/v2/{}/blobs/uploads/{}", name, uuid),
        )
        .header("Docker-Upload-UUID", uuid.as_str())
        .header("Range", "0-0")
        .header(header::CONTENT_LENGTH, 0)
        .body(axum::body::Body::empty())
        .unwrap()
}

async fn patch_upload(
    state: Arc<RegistryState>,
    name: &str,
    uuid: &str,
    _headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    match state.storage.patch_upload(uuid, body).await {
        None => registry_error(
            StatusCode::NOT_FOUND,
            ERR_BLOB_UPLOAD_UNKNOWN,
            "upload session not found",
        ),
        Some(offset) => Response::builder()
            .status(StatusCode::ACCEPTED)
            .header(
                header::LOCATION,
                format!("/v2/{}/blobs/uploads/{}", name, uuid),
            )
            .header("Docker-Upload-UUID", uuid)
            .header("Range", format!("0-{}", offset.saturating_sub(1)))
            .header(header::CONTENT_LENGTH, 0)
            .body(axum::body::Body::empty())
            .unwrap(),
    }
}

async fn complete_upload(
    state: Arc<RegistryState>,
    name: &str,
    uuid: &str,
    query: &HashMap<String, String>,
    body: Bytes,
) -> Response {
    let digest = match query.get("digest") {
        Some(d) => d.clone(),
        None => {
            return registry_error(
                StatusCode::BAD_REQUEST,
                ERR_DIGEST_INVALID,
                "digest query parameter required",
            )
        }
    };

    match state.storage.complete_upload(uuid, body, &digest).await {
        Err("upload session not found") => registry_error(
            StatusCode::NOT_FOUND,
            ERR_BLOB_UPLOAD_UNKNOWN,
            "upload session not found",
        ),
        Err(_) => registry_error(
            StatusCode::BAD_REQUEST,
            ERR_BLOB_UPLOAD_INVALID,
            "digest mismatch — blob corrupted",
        ),
        Ok((final_digest, _repo)) => Response::builder()
            .status(StatusCode::CREATED)
            .header(
                header::LOCATION,
                format!("/v2/{}/blobs/{}", name, final_digest),
            )
            .header("Docker-Content-Digest", final_digest.as_str())
            .header(header::CONTENT_LENGTH, 0)
            .body(axum::body::Body::empty())
            .unwrap(),
    }
}

async fn cancel_upload(
    state: Arc<RegistryState>,
    _name: &str,
    uuid: &str,
) -> Response {
    state.storage.cancel_upload(uuid).await;
    Response::builder()
        .status(StatusCode::NO_CONTENT)
        .body(axum::body::Body::empty())
        .unwrap()
}

// ── OCI 1.1 Referrers ─────────────────────────────────────────────────────────

async fn get_referrers(
    state: Arc<RegistryState>,
    _name: &str,
    digest: &str,
    query: &HashMap<String, String>,
) -> Response {
    let artifact_type_filter = query.get("artifactType").map(|s| s.as_str());
    let manifests = state
        .storage
        .get_referrers(digest, artifact_type_filter)
        .await;
    let resp = ReferrersResponse::new(manifests);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.oci.image.index.v1+json")
        .header("OCI-Referrers-State", "enabled")
        .body(axum::body::Body::from(serde_json::to_string(&resp).unwrap()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harbor::storage::compute_digest;
    use bytes::Bytes;

    fn make_state() -> Arc<RegistryState> {
        Arc::new(RegistryState::default())
    }

    #[tokio::test]
    async fn version_check_returns_200() {
        let resp = version_check().await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("docker-distribution-api-version").unwrap(),
            "registry/2.0"
        );
    }

    #[tokio::test]
    async fn catalog_empty() {
        let state = make_state();
        let resp = catalog(
            State(state),
            Query(PaginationQuery { n: None, last: None }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn blob_round_trip() {
        let state = make_state();
        let data = Bytes::from_static(b"hello blob");
        let digest = compute_digest(&data);
        state.storage.store_blob(digest.clone(), data.clone(), "testrepo").await;

        let resp = get_blob(Arc::clone(&state), "testrepo", &digest, false).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = get_blob(Arc::clone(&state), "testrepo", "sha256:notfound", false).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
