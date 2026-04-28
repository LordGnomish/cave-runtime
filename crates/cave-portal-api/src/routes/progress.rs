//! `/api/portal/progress` — high-level "what's done across the platform"
//! progress summary that the portal landing page renders.
//!
//! The implementation is intentionally a thin read-only router for now: the
//! actual roll-up is computed at the cave-portal layer from the parity_cache
//! the runtime populates at boot. This module exists so `cave-runtime`'s
//! main.rs can merge a router for the path even before per-tenant aggregation
//! lands.

use axum::{routing::get, Json, Router};

pub fn router() -> Router {
    Router::new().route("/api/portal/progress", get(progress))
}

async fn progress() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "endpoint": "progress",
        "ready": true,
        "note": "aggregate progress is computed in cave-portal::routes::parity_all"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use tower::util::ServiceExt;

    #[tokio::test]
    async fn router_serves_progress_endpoint() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/progress")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["endpoint"], "progress");
        assert_eq!(v["ready"], true);
    }

    #[tokio::test]
    async fn router_rejects_unknown_path() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/portal/missing")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn router_rejects_wrong_method() {
        let app = router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/portal/progress")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 405);
    }
}
