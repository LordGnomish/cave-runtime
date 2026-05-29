// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for the axum API route handlers.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cave_uptime::{AppState, router};
use std::sync::Arc;
use tower::ServiceExt; // for .oneshot()

#[tokio::test]
async fn test_health_route() {
    let state = Arc::new(AppState::new());
    let app = router(state);
    let req = Request::builder()
        .uri("/api/uptime/health")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_list_probes_empty() {
    let state = Arc::new(AppState::new());
    let app = router(state);
    let req = Request::builder()
        .uri("/api/uptime/probes")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.is_array() || json.get("probes").is_some());
}

#[tokio::test]
async fn test_create_probe() {
    let state = Arc::new(AppState::new());
    let app = router(state);

    let payload = serde_json::json!({
        "name": "API Health",
        "target_url": "https://api.example.com/health",
        "probe_type": "http",
        "interval_seconds": 60,
        "timeout_ms": 5000,
        "enabled": true
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/uptime/probes")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("id").is_some(), "response must have an id");
}

#[tokio::test]
async fn test_get_probe_not_found() {
    let state = Arc::new(AppState::new());
    let app = router(state);
    let req = Request::builder()
        .uri("/api/uptime/probes/00000000-0000-0000-0000-000000000000")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_then_get_probe() {
    let state = Arc::new(AppState::new());

    // Create
    let app = router(Arc::clone(&state));
    let payload = serde_json::json!({
        "name": "DB Port",
        "target_url": "db.internal:5432",
        "probe_type": "tcp",
        "interval_seconds": 30,
        "timeout_ms": 3000,
        "enabled": true
    });
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/uptime/probes")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let create_resp = app.oneshot(create_req).await.unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(create_resp.into_body(), usize::MAX).await.unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().expect("id field");

    // Get
    let app2 = router(Arc::clone(&state));
    let get_req = Request::builder()
        .uri(format!("/api/uptime/probes/{id}"))
        .body(Body::empty())
        .unwrap();
    let get_resp = app2.oneshot(get_req).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let get_body = axum::body::to_bytes(get_resp.into_body(), usize::MAX).await.unwrap();
    let probe: serde_json::Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(probe["name"].as_str(), Some("DB Port"));
}

#[tokio::test]
async fn test_delete_probe() {
    let state = Arc::new(AppState::new());

    // Create a probe
    let payload = serde_json::json!({
        "name": "Delete Me",
        "target_url": "http://example.com",
        "probe_type": "http",
        "interval_seconds": 60,
        "timeout_ms": 5000,
        "enabled": true
    });
    let app = router(Arc::clone(&state));
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/uptime/probes")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let create_resp = app.oneshot(create_req).await.unwrap();
    let body = axum::body::to_bytes(create_resp.into_body(), usize::MAX).await.unwrap();
    let created: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().expect("id field").to_string();

    // Delete it
    let app2 = router(Arc::clone(&state));
    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/uptime/probes/{id}"))
        .body(Body::empty())
        .unwrap();
    let del_resp = app2.oneshot(del_req).await.unwrap();
    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    // Verify gone
    let app3 = router(Arc::clone(&state));
    let get_req = Request::builder()
        .uri(format!("/api/uptime/probes/{id}"))
        .body(Body::empty())
        .unwrap();
    let get_resp = app3.oneshot(get_req).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
}
