// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for cave-chat HTTP API routes.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cave_chat::{AppState, router};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn make_app() -> axum::Router {
    let state = Arc::new(AppState::new());
    router(state)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// ── Health ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_ok() {
    let app = make_app().await;
    let req = Request::builder()
        .uri("/api/chat/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "ok");
}

// ── Channels ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_channel() {
    let app = make_app().await;
    let req = Request::builder()
        .method("POST")
        .uri("/api/chat/channels")
        .header("content-type", "application/json")
        .body(Body::from(json!({"name": "general", "channel_type": "public"}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "general");
    assert!(body["id"].as_str().is_some());
}

#[tokio::test]
async fn test_list_channels_empty() {
    let app = make_app().await;
    let req = Request::builder()
        .uri("/api/chat/channels")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_create_then_list_channel() {
    let state = Arc::new(AppState::new());
    let app = router(state.clone());

    // Create
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/chat/channels")
        .header("content-type", "application/json")
        .body(Body::from(json!({"name": "ops", "channel_type": "private"}).to_string()))
        .unwrap();
    let create_resp = app.clone().oneshot(create_req).await.unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);

    // List
    let list_req = Request::builder()
        .uri("/api/chat/channels")
        .body(Body::empty())
        .unwrap();
    let list_resp = app.oneshot(list_req).await.unwrap();
    let body = body_json(list_resp).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
}

// ── Messages ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_post_and_get_messages() {
    let state = Arc::new(AppState::new());
    let app = router(state.clone());

    // Create channel first
    let ch_req = Request::builder()
        .method("POST")
        .uri("/api/chat/channels")
        .header("content-type", "application/json")
        .body(Body::from(json!({"name": "dev", "channel_type": "public"}).to_string()))
        .unwrap();
    let ch_resp = app.clone().oneshot(ch_req).await.unwrap();
    let ch_body = body_json(ch_resp).await;
    let channel_id = ch_body["id"].as_str().unwrap().to_string();

    // Post message
    let msg_req = Request::builder()
        .method("POST")
        .uri(format!("/api/chat/channels/{}/messages", channel_id))
        .header("content-type", "application/json")
        .body(Body::from(json!({"author_id": "alice", "content": "hello there"}).to_string()))
        .unwrap();
    let msg_resp = app.clone().oneshot(msg_req).await.unwrap();
    assert_eq!(msg_resp.status(), StatusCode::CREATED);
    let msg_body = body_json(msg_resp).await;
    assert_eq!(msg_body["content"], "hello there");

    // Get messages
    let get_req = Request::builder()
        .uri(format!("/api/chat/channels/{}/messages", channel_id))
        .body(Body::empty())
        .unwrap();
    let get_resp = app.oneshot(get_req).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let msgs = body_json(get_resp).await;
    assert_eq!(msgs.as_array().unwrap().len(), 1);
}

// ── Search ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_search_messages() {
    let state = Arc::new(AppState::new());
    let app = router(state.clone());

    // Create channel
    let ch_req = Request::builder()
        .method("POST")
        .uri("/api/chat/channels")
        .header("content-type", "application/json")
        .body(Body::from(json!({"name": "search-test", "channel_type": "public"}).to_string()))
        .unwrap();
    let ch_body = body_json(app.clone().oneshot(ch_req).await.unwrap()).await;
    let cid = ch_body["id"].as_str().unwrap().to_string();

    // Post two messages
    for content in &["deployment failed", "all systems go"] {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/chat/channels/{}/messages", cid))
            .header("content-type", "application/json")
            .body(Body::from(json!({"author_id": "bot", "content": content}).to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    // Search
    let search_req = Request::builder()
        .uri("/api/chat/search?q=deployment")
        .body(Body::empty())
        .unwrap();
    let results = body_json(app.oneshot(search_req).await.unwrap()).await;
    assert_eq!(results.as_array().unwrap().len(), 1);
    assert!(results[0]["content"].as_str().unwrap().contains("deployment"));
}

// ── Presence ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_set_and_get_presence() {
    let state = Arc::new(AppState::new());
    let app = router(state.clone());

    // Set presence
    let set_req = Request::builder()
        .method("PUT")
        .uri("/api/chat/presence/alice")
        .header("content-type", "application/json")
        .body(Body::from(json!({"status": "online"}).to_string()))
        .unwrap();
    let set_resp = app.clone().oneshot(set_req).await.unwrap();
    assert_eq!(set_resp.status(), StatusCode::OK);

    // Get presence
    let get_req = Request::builder()
        .uri("/api/chat/presence/alice")
        .body(Body::empty())
        .unwrap();
    let get_resp = app.oneshot(get_req).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = body_json(get_resp).await;
    assert_eq!(body["status"], "online");
}

// ── Stats ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_stats_endpoint() {
    let app = make_app().await;
    let req = Request::builder()
        .uri("/api/chat/stats")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["total_channels"].as_u64().is_some());
    assert!(body["total_messages"].as_u64().is_some());
}
