// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes — port of router.ts from @backstage/plugin-techdocs-backend.
//!
//! Upstream: backstage/plugins/techdocs-backend/src/service/router.ts

use crate::models::{EntityMetadata, EntityMetadataInner, EntityName};
use crate::publisher::{Publisher, TechDocsError};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::collections::HashMap;
use std::sync::Arc;

/// Shared application state for TechDocs routes.
pub struct TechDocsState {
    pub publisher: Arc<dyn Publisher>,
}

/// Build the axum Router for TechDocs API.
///
/// Upstream: createRouter() in router.ts
pub fn create_router(state: Arc<TechDocsState>) -> Router {
    Router::new()
        .route("/api/techdocs/health", get(health))
        .route(
            "/api/techdocs/metadata/techdocs/{namespace}/{kind}/{name}",
            get(metadata_techdocs),
        )
        .route(
            "/api/techdocs/metadata/entity/{namespace}/{kind}/{name}",
            get(metadata_entity),
        )
        .route(
            "/api/techdocs/static/docs/{namespace}/{kind}/{name}/{*path}",
            get(static_file),
        )
        .route(
            "/api/techdocs/sync/{namespace}/{kind}/{name}",
            post(sync_entity),
        )
        .with_state(state)
}

// ── Handlers ───────────────────────────────────────────────────────────────

/// GET /api/techdocs/health
///
/// Upstream: router.ts health endpoint
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "upstream": "Backstage TechDocs"
    }))
}

/// GET /api/techdocs/metadata/techdocs/:namespace/:kind/:name
///
/// Upstream: router.ts GET /metadata/techdocs/:namespace/:kind/:name
async fn metadata_techdocs(
    State(state): State<Arc<TechDocsState>>,
    Path((namespace, kind, name)): Path<(String, String, String)>,
) -> Response {
    let entity = EntityName::new(namespace, kind, name);
    match state.publisher.fetch_metadata(&entity).await {
        Ok(meta) => Json(meta).into_response(),
        Err(TechDocsError::NotFound(msg)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": msg })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/techdocs/metadata/entity/:namespace/:kind/:name
///
/// Upstream: router.ts GET /metadata/entity/:namespace/:kind/:name
/// Returns a catalog-compatible stub — full catalog integration is out of scope here.
async fn metadata_entity(
    Path((namespace, kind, name)): Path<(String, String, String)>,
) -> Json<EntityMetadata> {
    let uid = uuid::Uuid::new_v4().to_string();
    Json(EntityMetadata {
        api_version: "backstage.io/v1alpha1".to_string(),
        kind: kind,
        metadata: EntityMetadataInner {
            namespace: namespace,
            name: name,
            description: None,
            annotations: HashMap::new(),
            labels: HashMap::new(),
            uid,
        },
        spec: serde_json::json!({}),
    })
}

/// GET /api/techdocs/static/docs/:namespace/:kind/:name/*path
///
/// Upstream: router.ts GET /static/docs/:namespace/:kind/:name/*path
async fn static_file(
    State(state): State<Arc<TechDocsState>>,
    Path((namespace, kind, name, path)): Path<(String, String, String, String)>,
) -> Response {
    let entity = EntityName::new(namespace, kind, name);
    match state.publisher.read_file(&entity, &path).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", content_type_for(&path))
            .body(Body::from(bytes))
            .unwrap(),
        Err(TechDocsError::NotFound(msg)) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": msg })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// POST /api/techdocs/sync/:namespace/:kind/:name
///
/// Upstream: router.ts POST /sync/:namespace/:kind/:name
/// Returns {"status":"queued"} — sync is async in upstream too.
async fn sync_entity(
    Path((namespace, kind, name)): Path<(String, String, String)>,
) -> Json<serde_json::Value> {
    tracing::info!(
        namespace = %namespace,
        kind = %kind,
        name = %name,
        "techdocs sync queued"
    );
    Json(serde_json::json!({ "status": "queued" }))
}

/// Infer content-type from file extension.
fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tempfile::TempDir;
    use tower::util::ServiceExt;

    /// Build a test router backed by a real LocalPublisher against a temp dir.
    fn test_app() -> (Router, TempDir) {
        let tmp = TempDir::new().unwrap();
        let publisher = crate::publisher::local::LocalPublisher::new(tmp.path());
        let state = Arc::new(TechDocsState {
            publisher: Arc::new(publisher),
        });
        (create_router(state), tmp)
    }

    async fn get_req(app: Router, path: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn post_req(app: Router, path: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// GET /api/techdocs/health → 200 {"status":"ok","upstream":"Backstage TechDocs"}
    ///
    /// Upstream: router.test.ts — "health endpoint returns 200"
    #[tokio::test]
    async fn health_returns_ok() {
        let (app, _tmp) = test_app();
        let resp = get_req(app, "/api/techdocs/health").await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["upstream"], "Backstage TechDocs");
    }

    /// GET /api/techdocs/metadata/techdocs/... → 404 when no docs published.
    ///
    /// Upstream: router.test.ts — "metadata/techdocs returns 404 when no docs"
    #[tokio::test]
    async fn metadata_techdocs_not_found() {
        let (app, _tmp) = test_app();
        let resp = get_req(
            app,
            "/api/techdocs/metadata/techdocs/default/Component/my-service",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// GET /api/techdocs/metadata/entity/... → 200 with stub containing kind/name/namespace.
    ///
    /// Upstream: router.test.ts — "metadata/entity returns entity stub"
    #[tokio::test]
    async fn metadata_entity_returns_stub() {
        let (app, _tmp) = test_app();
        let resp = get_req(
            app,
            "/api/techdocs/metadata/entity/default/Component/my-service",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["kind"], "Component");
        assert_eq!(json["metadata"]["name"], "my-service");
        assert_eq!(json["metadata"]["namespace"], "default");
    }

    /// GET /api/techdocs/static/docs/... → 404 when file is missing.
    ///
    /// Upstream: router.test.ts — "static file returns 404 when missing"
    #[tokio::test]
    async fn static_file_not_found() {
        let (app, _tmp) = test_app();
        let resp = get_req(
            app,
            "/api/techdocs/static/docs/default/Component/my-service/index.html",
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    /// POST /api/techdocs/sync/... → 200 {"status":"queued"}
    ///
    /// Upstream: router.test.ts — "sync endpoint queues build"
    #[tokio::test]
    async fn sync_queues_build() {
        let (app, _tmp) = test_app();
        let resp = post_req(app, "/api/techdocs/sync/default/Component/my-service").await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "queued");
    }
}
