//! Integration tests for `OllamaExtrasClient` against a real-in-process axum mock server.
//!
//! Each test spins up an ephemeral HTTP server on a random port and asserts that
//! `OllamaExtrasClient` correctly serialises requests and deserialises responses.

use axum::{Router, routing::{get, post, delete}};
use cave_local_llm::ollama_extras::*;
use serde_json::json;
use tokio::net::TcpListener;

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn spawn_mock_server(router: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
     });
    format!("http://{addr}")
}

// ── POST /api/show ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_show_parses_response() {
    let app = Router::new().route(
         "/api/show",
        post(|| async {
            axum::Json(json!({
                 "parent_model": "",
                 "format": "f16",
                 "family": "qwen2.5-coder",
                 "families": ["qwen2.5-coder"],
                 "parameter_size": "3.8B",
                 "license": "Apache-2.0",
                 "modified_at": "2024-01-01T00:00:00Z",
                 "details": {
                     "parent_model": "",
                     "format": "f16",
                     "family": "qwen2.5-coder",
                     "families": ["qwen2.5-coder"],
                     "parameter_size": "3.8B",
                     "quantization_level": "Q4_K_M"
                 }
             }))
         }),
     );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaExtrasClient::new(base_url);
    let resp = client.show(ShowRequest {
        model: "qwen2.5-coder:32b".into(),
     }).await.expect("show should succeed");
    assert_eq!(resp.format, Some("f16".into()));
    assert_eq!(resp.family, Some("qwen2.5-coder".into()));
    assert_eq!(resp.parameter_size, Some("3.8B".into()));
    assert_eq!(resp.license, Some("Apache-2.0".into()));
    assert!(!resp.families.clone().unwrap().is_empty());
    assert_eq!(resp.details.as_ref().unwrap().quantization_level, Some("Q4_K_M".into()));
}

// ── POST /api/copy ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_copy_returns_success() {
    let app = Router::new().route(
         "/api/copy",
        post(|| async {
            axum::Json(json!({ "status": "success" }))
         }),
     );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaExtrasClient::new(base_url);
    let resp = client.copy(CopyRequest {
        source: "qwen2.5-coder:32b".into(),
        destination: "qwen2.5-coder:small".into(),
     }).await.expect("copy should succeed");
    assert_eq!(resp.status, "success");
}

// ── POST /api/delete ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_returns_success() {
    let app = Router::new().route(
         "/api/delete",
        delete(|| async {
            axum::Json(json!({ "status": "success" }))
         }),
     );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaExtrasClient::new(base_url);
    let resp = client.delete(DeleteRequest {
        model: "qwen2.5-coder:32b".into(),
     }).await.expect("delete should succeed");
    assert_eq!(resp.status, "success");
}

// ── POST /api/embed ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_embed_returns_embeddings() {
    let app = Router::new().route(
         "/api/embed",
        post(|| async {
            axum::Json(json!({
                 "model": "nomic-embed-text",
                 "embeddings": [
                     [0.1, 0.2, 0.3, 0.4, 0.5],
                     [0.5, 0.4, 0.3, 0.2, 0.1]
                 ],
                 "total_duration": 1234567
             }))
         }),
     );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaExtrasClient::new(base_url);
    let resp = client.embed(EmbedRequest {
        model: "nomic-embed-text".into(),
        input: vec!["hello".into(), "world".into()],
        truncate: Some(true),
     }).await.expect("embed should succeed");
    assert_eq!(resp.model.as_ref().unwrap(), "nomic-embed-text");
    assert_eq!(resp.embeddings.len(), 2);
    assert_eq!(resp.embeddings[0], vec![0.1, 0.2, 0.3, 0.4, 0.5]);
    assert_eq!(resp.total_duration, Some(1_234_567));
    assert_eq!(resp.embeddings[1], vec![0.5, 0.4, 0.3, 0.2, 0.1]);
}

// ── GET /api/ps (list running) ───────────────────────────────────────────────

#[tokio::test]
async fn test_list_running_returns_models() {
    let app = Router::new().route(
         "/api/ps",
        get(|| async {
            axum::Json(json!({
                 "models": [
                     {
                         "name": "qwen2.5-coder:32b",
                         "model": "qwen2.5-coder:32b",
                         "size": 20_000_000_000_u64,
                         "digest": "abc123",
                         "expires_at": "2024-01-02T00:00:00Z",
                         "ttl": 86400000000000i64
                     }
                 ]
             }))
         }),
     );

    let base_url = spawn_mock_server(app).await;
    let resp = OllamaExtrasClient::new(base_url).list_running().await.expect("ps should succeed");
    assert_eq!(resp.models.len(), 1);
    assert_eq!(resp.models[0].name.as_ref().unwrap(), "qwen2.5-coder:32b");
    assert_eq!(resp.models[0].size, Some(20_000_000_000));
    assert_eq!(resp.models[0].expires_at, Some("2024-01-02T00:00:00Z".into()));
}

#[tokio::test]
async fn test_list_running_empty() {
    let app = Router::new().route(
         "/api/ps",
        get(|| async { axum::Json(json!({ "models": [] })) }),
     );

    let base_url = spawn_mock_server(app).await;
    let resp = OllamaExtrasClient::new(base_url).list_running().await.unwrap();
    assert!(resp.models.is_empty());
}

// ── Error propagation ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_show_error_returns_api_error() {
    let app = Router::new().route(
         "/api/show",
        post(|| async {
             (
                axum::http::StatusCode::BAD_REQUEST,
                 "model not found",
             )
         }),
     );

    let base_url = spawn_mock_server(app).await;
    let err = OllamaExtrasClient::new(base_url)
         .show(ShowRequest { model: "nonexistent".into() })
         .await
         .unwrap_err();
    match err {
        OllamaExtrasError::Api { status, body } => {
            assert_eq!(status, 400);
            assert!(body.contains("model not found"));
         }
        other => panic!("expected OllamaExtrasError::Api, got {other:?}"),
     }
}

#[tokio::test]
async fn test_connection_refused_returns_http_error() {
     // Port 1 is reserved/closed — connection will be refused.
    let client = OllamaExtrasClient::new("http://127.0.0.1:1");
    let err = client.list_running().await.unwrap_err();
    assert!(
        matches!(err, OllamaExtrasError::Http(_)),
         "expected Http error, got {err:?}"
     );
}
