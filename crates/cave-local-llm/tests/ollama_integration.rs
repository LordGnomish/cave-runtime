// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `OllamaClient` against a real-in-process axum mock server.
//!
//! Each test spins up an ephemeral HTTP server on a random port and asserts that
//! `OllamaClient` correctly serialises requests and deserialises responses.

use axum::{Router, routing::get, routing::post};
use cave_local_llm::ollama::{ChatMessage, ChatRequest, GenerateRequest, OllamaClient};
use futures::StreamExt;
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

// ── /api/tags ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_models_parses_response() {
    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            axum::Json(json!({
                "models": [
                    {
                        "name": "qwen2.5-coder:32b",
                        "modified_at": "2024-01-01T00:00:00Z",
                        "size": 20_000_000_000_u64,
                        "digest": "abc123"
                    }
                ]
            }))
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaClient::new(base_url);

    let models = client.list_models().await.expect("list_models should succeed");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "qwen2.5-coder:32b");
    assert_eq!(models[0].size, 20_000_000_000);
}

#[tokio::test]
async fn test_list_models_empty_list() {
    let app = Router::new().route(
        "/api/tags",
        get(|| async { axum::Json(json!({ "models": [] })) }),
    );

    let base_url = spawn_mock_server(app).await;
    let models = OllamaClient::new(base_url).list_models().await.unwrap();
    assert!(models.is_empty());
}

// ── /api/generate (non-streaming) ────────────────────────────────────────────

#[tokio::test]
async fn test_generate_returns_response() {
    let app = Router::new().route(
        "/api/generate",
        post(|| async {
            axum::Json(json!({
                "model": "qwen2.5-coder:32b",
                "created_at": "2024-01-01T00:00:00Z",
                "response": "fn foo() { 42 }",
                "done": true,
                "total_duration": 1_000_000_u64
            }))
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaClient::new(base_url);
    let req = GenerateRequest {
        model: "qwen2.5-coder:32b".into(),
        prompt: "write a function".into(),
        stream: Some(false),
        options: None,
        keep_alive: None,
    };

    let resp = client.generate(req).await.expect("generate should succeed");
    assert_eq!(resp.response, "fn foo() { 42 }");
    assert!(resp.done);
    assert_eq!(resp.model, "qwen2.5-coder:32b");
}

// ── /api/generate (streaming) ─────────────────────────────────────────────────

#[tokio::test]
async fn test_generate_stream_collects_all_chunks() {
    let ndjson = concat!(
        "{\"model\":\"qwen2.5-coder:32b\",\"created_at\":\"2024-01-01T00:00:00Z\",\"response\":\"fn \",\"done\":false}\n",
        "{\"model\":\"qwen2.5-coder:32b\",\"created_at\":\"2024-01-01T00:00:00Z\",\"response\":\"foo\",\"done\":false}\n",
        "{\"model\":\"qwen2.5-coder:32b\",\"created_at\":\"2024-01-01T00:00:00Z\",\"response\":\"() {}\",\"done\":true,\"total_duration\":500000}\n",
    );

    let app = Router::new().route(
        "/api/generate",
        post(move || async move {
            (
                [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
                ndjson,
            )
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let client = OllamaClient::new(base_url);
    let req = GenerateRequest {
        model: "qwen2.5-coder:32b".into(),
        prompt: "write a function".into(),
        stream: Some(true),
        options: None,
        keep_alive: None,
    };

    let mut stream = client.generate_stream(req).await.expect("stream should open");
    let mut chunks = Vec::new();
    while let Some(item) = stream.next().await {
        chunks.push(item.expect("chunk should deserialise"));
    }

    assert_eq!(chunks.len(), 3, "expected 3 chunks");
    assert_eq!(chunks[0].response, "fn ");
    assert_eq!(chunks[1].response, "foo");
    assert_eq!(chunks[2].response, "() {}");
    assert!(chunks[2].done);
    assert_eq!(chunks[2].total_duration, Some(500_000));
}

#[tokio::test]
async fn test_generate_stream_skips_empty_lines() {
    let ndjson = concat!(
        "{\"model\":\"m\",\"created_at\":\"2024-01-01T00:00:00Z\",\"response\":\"a\",\"done\":false}\n",
        "\n",
        "{\"model\":\"m\",\"created_at\":\"2024-01-01T00:00:00Z\",\"response\":\"b\",\"done\":true}\n",
    );

    let app = Router::new().route(
        "/api/generate",
        post(move || async move {
            (
                [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
                ndjson,
            )
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let stream = OllamaClient::new(base_url)
        .generate_stream(GenerateRequest {
            model: "m".into(),
            prompt: "p".into(),
            stream: Some(true),
            options: None,
            keep_alive: None,
        })
        .await
        .unwrap();

    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;
    assert_eq!(chunks.len(), 2, "empty line must be skipped");
}

// ── /api/chat (non-streaming) ─────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_returns_response() {
    let app = Router::new().route(
        "/api/chat",
        post(|| async {
            axum::Json(json!({
                "model": "qwen2.5-coder:32b",
                "created_at": "2024-01-01T00:00:00Z",
                "message": { "role": "assistant", "content": "Hello from Cave!" },
                "done": true
            }))
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let req = ChatRequest {
        model: "qwen2.5-coder:32b".into(),
        messages: vec![ChatMessage { role: "user".into(), content: "hi".into() }],
        stream: Some(false),
        options: None,
    };

    let resp = OllamaClient::new(base_url).chat(req).await.unwrap();
    assert_eq!(resp.message.role, "assistant");
    assert_eq!(resp.message.content, "Hello from Cave!");
    assert!(resp.done);
}

// ── /api/chat (streaming) ────────────────────────────────────────────────────

#[tokio::test]
async fn test_chat_stream_collects_chunks() {
    let ndjson = concat!(
        "{\"model\":\"m\",\"created_at\":\"2024-01-01T00:00:00Z\",\"message\":{\"role\":\"assistant\",\"content\":\"Hi\"},\"done\":false}\n",
        "{\"model\":\"m\",\"created_at\":\"2024-01-01T00:00:00Z\",\"message\":{\"role\":\"assistant\",\"content\":\"!\"},\"done\":true}\n",
    );

    let app = Router::new().route(
        "/api/chat",
        post(move || async move {
            (
                [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
                ndjson,
            )
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let req = ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage { role: "user".into(), content: "hey".into() }],
        stream: Some(true),
        options: None,
    };

    let stream = OllamaClient::new(base_url).chat_stream(req).await.unwrap();
    let chunks: Vec<_> = stream.collect::<Vec<_>>().await;

    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].as_ref().unwrap().message.content, "Hi");
    assert!(chunks[1].as_ref().unwrap().done);
}

// ── Error propagation ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_api_error_propagates_status_and_body() {
    let app = Router::new().route(
        "/api/tags",
        get(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "ollama internal error",
            )
        }),
    );

    let base_url = spawn_mock_server(app).await;
    let err = OllamaClient::new(base_url).list_models().await.unwrap_err();

    match err {
        cave_local_llm::ollama::OllamaError::Api { status, body } => {
            assert_eq!(status, 500);
            assert!(body.contains("ollama internal error"));
        }
        other => panic!("expected OllamaError::Api, got {other:?}"),
    }
}

#[tokio::test]
async fn test_connection_refused_returns_http_error() {
    // Port 1 is reserved/closed on all platforms — connection will be refused.
    let client = OllamaClient::new("http://127.0.0.1:1");
    let err = client.list_models().await.unwrap_err();
    assert!(
        matches!(err, cave_local_llm::ollama::OllamaError::Http(_)),
        "expected Http error, got {err:?}"
    );
}
