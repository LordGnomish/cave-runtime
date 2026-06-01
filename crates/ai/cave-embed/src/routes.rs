// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes — OpenAI-compatible `/v1/embeddings` + `/v1/models`, the
//! Cohere/infinity `/rerank` endpoint, a JSON model catalog, and an
//! `/admin/embed` server-rendered status page.

use crate::error::EmbedError;
use crate::openai::{EmbeddingRequest, EmbeddingService};
use crate::rerank::{RerankRequest, RerankService};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde_json::json;
use std::sync::Arc;

/// Shared service state for the embedding server.
pub struct EmbedState {
    /// OpenAI-compatible embedding service.
    pub embeddings: EmbeddingService,
    /// Rerank service.
    pub rerank: RerankService,
}

impl Default for EmbedState {
    fn default() -> Self {
        Self {
            embeddings: EmbeddingService::default(),
            rerank: RerankService::default(),
        }
    }
}

fn status_for(err: &EmbedError) -> StatusCode {
    match err {
        EmbedError::UnknownModel(_) => StatusCode::NOT_FOUND,
        EmbedError::EmptyInput
        | EmbedError::InvalidDimensions { .. }
        | EmbedError::InvalidArgument(_)
        | EmbedError::ShapeMismatch { .. } => StatusCode::BAD_REQUEST,
        EmbedError::Degenerate(_) => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

fn error_response(err: EmbedError) -> axum::response::Response {
    let status = status_for(&err);
    (
        status,
        Json(json!({"error": {"message": err.to_string(), "type": "embed_error"}})),
    )
        .into_response()
}

/// Build the cave-embed router.
pub fn create_router(state: Arc<EmbedState>) -> Router {
    Router::new()
        // OpenAI-compatible surface
        .route("/v1/embeddings", post(embeddings))
        .route("/v1/models", get(list_models))
        // Cohere/infinity rerank
        .route("/rerank", post(rerank))
        // Admin / introspection
        .route("/admin/embed", get(admin_page))
        .route("/api/embed/models", get(model_cards))
        .route("/api/embed/health", get(health))
        .with_state(state)
}

async fn embeddings(
    State(state): State<Arc<EmbedState>>,
    Json(req): Json<EmbeddingRequest>,
) -> axum::response::Response {
    match state.embeddings.embed(&req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => error_response(e),
    }
}

async fn rerank(
    State(state): State<Arc<EmbedState>>,
    Json(req): Json<RerankRequest>,
) -> axum::response::Response {
    match state.rerank.rerank(&req).await {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => error_response(e),
    }
}

async fn list_models(State(state): State<Arc<EmbedState>>) -> impl IntoResponse {
    let data: Vec<_> = state
        .embeddings
        .catalog()
        .ids()
        .into_iter()
        .map(|id| json!({"id": id, "object": "model", "owned_by": "cave-embed"}))
        .collect();
    Json(json!({"object": "list", "data": data}))
}

async fn model_cards(State(state): State<Arc<EmbedState>>) -> impl IntoResponse {
    let catalog = state.embeddings.catalog();
    let cards: Vec<_> = catalog
        .ids()
        .into_iter()
        .filter_map(|id| catalog.get(id))
        .map(|c| {
            json!({
                "id": c.id,
                "family": format!("{:?}", c.family),
                "dims": c.dims,
                "max_seq_len": c.max_seq_len,
                "pooling": format!("{:?}", c.pooling),
                "normalize": c.normalize,
                "matryoshka": c.matryoshka_dims,
            })
        })
        .collect();
    Json(json!({"models": cards}))
}

async fn health() -> impl IntoResponse {
    Json(json!({"status": "ok", "version": crate::UPSTREAM_VERSION}))
}

async fn admin_page(State(state): State<Arc<EmbedState>>) -> Html<String> {
    let catalog = state.embeddings.catalog();
    let mut rows = String::new();
    for id in catalog.ids() {
        if let Some(c) = catalog.get(id) {
            rows.push_str(&format!(
                "<tr><td>{}</td><td>{:?}</td><td>{}</td><td>{:?}</td><td>{}</td></tr>",
                c.id, c.family, c.dims, c.pooling, c.normalize
            ));
        }
    }
    Html(format!(
        "<!doctype html><html><head><title>cave-embed</title></head><body>\
         <h1>cave-embed</h1><p>OpenAI-compatible embeddings + rerank · infinity parity \
         v{ver}</p><table border=1><thead><tr><th>model</th><th>family</th><th>dims</th>\
         <th>pooling</th><th>normalize</th></tr></thead><tbody>{rows}</tbody></table>\
         </body></html>",
        ver = crate::UPSTREAM_VERSION,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn body_string(resp: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn app() -> Router {
        create_router(Arc::new(EmbedState::default()))
    }

    #[tokio::test]
    async fn list_models_lists_catalog() {
        let resp = app()
            .oneshot(Request::get("/v1/models").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_string(resp).await.contains("all-MiniLM-L6-v2"));
    }

    #[tokio::test]
    async fn embeddings_endpoint_returns_vectors() {
        let payload =
            r#"{"input":"hello world","model":"sentence-transformers/all-MiniLM-L6-v2"}"#;
        let resp = app()
            .oneshot(
                Request::post("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body_string(resp).await;
        assert!(b.contains("\"object\":\"list\""));
        assert!(b.contains("\"embedding\""));
    }

    #[tokio::test]
    async fn unknown_model_is_404() {
        let payload = r#"{"input":"x","model":"nope/x"}"#;
        let resp = app()
            .oneshot(
                Request::post("/v1/embeddings")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn rerank_endpoint_sorts() {
        let payload = r#"{"model":"rerank-ref","query":"red blue green","documents":["apple","red blue green"],"return_documents":true}"#;
        let resp = app()
            .oneshot(
                Request::post("/rerank")
                    .header("content-type", "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body_string(resp).await;
        assert!(b.contains("\"results\""));
        assert!(b.contains("relevance_score"));
    }

    #[tokio::test]
    async fn admin_page_renders() {
        let resp = app()
            .oneshot(Request::get("/admin/embed").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_string(resp).await.contains("cave-embed"));
    }

    #[tokio::test]
    async fn health_ok() {
        let resp = app()
            .oneshot(Request::get("/api/embed/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
