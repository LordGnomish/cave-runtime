// SPDX-License-Identifier: AGPL-3.0-or-later
//! HTTP surface: the axum router cave-runtime merges to expose RAG endpoints.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use cave_rag::routes::{router, RagState};
use serde_json::Value;
use tower::ServiceExt; // for `oneshot`

async fn post(uri: &str, body: &str) -> (StatusCode, Value) {
    let app = router(Arc::new(RagState::default()));
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn health_reports_module_metadata() {
    let app = router(Arc::new(RagState::default()));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/rag/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["status"], "healthy");
    assert_eq!(v["module"], "cave-rag");
}

#[tokio::test]
async fn graph_extract_endpoint_returns_entities_and_communities() {
    let (status, v) = post(
        "/api/rag/graph/extract",
        r#"{"text":"Alice works with Bob at Acme. Carol leads Globex with Dave."}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entities = v["entities"].as_array().unwrap();
    assert!(entities.iter().any(|e| e == "Alice"));
    assert!(entities.iter().any(|e| e == "Globex"));
    assert_eq!(v["communities"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn graph_search_endpoint_returns_neighborhood() {
    let (status, v) = post(
        "/api/rag/graph/search",
        r#"{"text":"Alice works with Bob at Acme. Carol leads Globex with Dave.","query":"tell me about Alice","hops":1}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ctx = v["context"].as_array().unwrap();
    assert!(ctx.iter().any(|e| e == "Bob"));
    assert!(!ctx.iter().any(|e| e == "Dave"));
}

#[tokio::test]
async fn split_endpoint_chunks_text() {
    let (status, v) = post(
        "/api/rag/split",
        r#"{"text":"The first sentence. The second sentence. The third one here.","size":25}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(v["chunk_count"].as_u64().unwrap() >= 2);
}
