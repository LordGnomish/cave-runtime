//! HTTP routes for cave-permission.
//!
//! Upstream: backstage/plugins/permission-backend/src/service/router.ts

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

use crate::{
    models::{
        AuthorizeRequest, AuthorizeResponse, AuthorizeResult, EvaluatePermissionResponse,
        PolicyDecision,
    },
    policy::{BackstagePrincipal, PolicyQuery},
    PermissionState,
};

pub fn create_router(state: Arc<PermissionState>) -> Router {
    Router::new()
        .route("/api/permission/health", get(health))
        .route("/api/permission/authorize", post(authorize))
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "upstream": "Backstage permission-backend"
    }))
}

/// POST /api/permission/authorize
///
/// Upstream: router.ts — evaluates each item in the batch through the policy
/// and returns a matching response array with the same IDs.
async fn authorize(
    State(state): State<Arc<PermissionState>>,
    Json(req): Json<AuthorizeRequest>,
) -> Json<AuthorizeResponse> {
    let mut items = Vec::with_capacity(req.items.len());

    for item in &req.items {
        let query = PolicyQuery {
            permission: item.permission.clone(),
            principal: None,
        };

        let principal: Option<BackstagePrincipal> = None;
        let decision = state
            .policy
            .handle(&query, principal.as_ref())
            .await;

        let result = match decision {
            PolicyDecision::Allow => AuthorizeResult::Allow,
            PolicyDecision::Deny => AuthorizeResult::Deny,
        };

        items.push(EvaluatePermissionResponse {
            id: item.id.clone(),
            result,
        });
    }

    Json(AuthorizeResponse { items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::util::ServiceExt;

    fn test_app() -> Router {
        create_router(Arc::new(PermissionState::default()))
    }

    async fn post_json(
        app: Router,
        path: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
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

    #[tokio::test]
    async fn health_returns_ok() {
        let resp = get_req(test_app(), "/api/permission/health").await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["upstream"], "Backstage permission-backend");
    }

    #[tokio::test]
    async fn authorize_allow_all_policy() {
        let body = serde_json::json!({
            "items": [
                {
                    "id": "item-1",
                    "permission": {
                        "name": "catalog.entity.read",
                        "attributes": { "action": "read" }
                    },
                    "resource_ref": null
                },
                {
                    "id": "item-2",
                    "permission": {
                        "name": "catalog.entity.delete",
                        "attributes": { "action": "delete" }
                    },
                    "resource_ref": null
                }
            ]
        });

        let resp = post_json(test_app(), "/api/permission/authorize", body).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["items"][0]["result"], "ALLOW");
        assert_eq!(json["items"][1]["result"], "ALLOW");
    }

    #[tokio::test]
    async fn authorize_returns_ids_in_response() {
        let body = serde_json::json!({
            "items": [
                {
                    "id": "unique-id-abc",
                    "permission": {
                        "name": "catalog.entity.read",
                        "attributes": { "action": "read" }
                    },
                    "resource_ref": null
                },
                {
                    "id": "unique-id-xyz",
                    "permission": {
                        "name": "catalog.entity.create",
                        "attributes": { "action": "create" }
                    },
                    "resource_ref": null
                }
            ]
        });

        let resp = post_json(test_app(), "/api/permission/authorize", body).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(json["items"][0]["id"], "unique-id-abc");
        assert_eq!(json["items"][1]["id"], "unique-id-xyz");
    }

    #[tokio::test]
    async fn authorize_empty_batch() {
        let body = serde_json::json!({ "items": [] });

        let resp = post_json(test_app(), "/api/permission/authorize", body).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let items = json["items"].as_array().unwrap();
        assert!(items.is_empty());
    }
}
