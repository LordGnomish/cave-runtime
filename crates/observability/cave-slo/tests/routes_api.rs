// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration-style tests for cave-slo HTTP routes.
//! Written FIRST per TDD.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cave_slo::{State, router};
use cave_slo::models::{MetricType, SloStatus};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt; // for `.oneshot()`

fn new_state() -> Arc<State> {
    State::new()
}

async fn call(state: Arc<State>, req: Request<Body>) -> (StatusCode, Value) {
    let app = router(state);
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, json)
}

// ── GET /api/slo/health ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_returns_ok() {
    let state = new_state();
    let req = Request::builder()
        .uri("/api/slo/health")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "ok");
    assert_eq!(json["module"], "cave-slo");
}

// ── POST /api/slo/slos ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_slo_returns_201() {
    let state = new_state();
    let body = serde_json::json!({
        "name": "api-availability",
        "description": "API must be up 99.9%",
        "target_percentage": 99.9,
        "window_days": 30,
        "metric_type": "availability"
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/slo/slos")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::CREATED, "body={json}");
    assert_eq!(json["name"], "api-availability");
    assert!(json["id"].is_string());
}

// ── GET /api/slo/slos ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_slos_empty() {
    let state = new_state();
    let req = Request::builder()
        .uri("/api/slo/slos")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_slos_after_create() {
    let state = new_state();
    // Create one
    let body = serde_json::json!({
        "name": "my-slo", "description": "d", "target_percentage": 99.0,
        "window_days": 7, "metric_type": "availability"
    });
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/slo/slos")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let app = router(state.clone());
    app.oneshot(create_req).await.unwrap();

    // List
    let list_req = Request::builder()
        .uri("/api/slo/slos")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, list_req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json.as_array().unwrap().len(), 1);
    assert_eq!(json[0]["name"], "my-slo");
}

// ── GET /api/slo/slos/:id ────────────────────────────────────────────────────

#[tokio::test]
async fn test_get_slo_not_found() {
    let state = new_state();
    let req = Request::builder()
        .uri("/api/slo/slos/00000000-0000-0000-0000-000000000000")
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_slo_by_id() {
    let state = new_state();
    // Create
    let body = serde_json::json!({
        "name": "find-me", "description": "d", "target_percentage": 99.5,
        "window_days": 14, "metric_type": "latency"
    });
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/slo/slos")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let (_, created) = call(state.clone(), create_req).await;
    let id = created["id"].as_str().unwrap().to_string();

    // Retrieve by ID
    let get_req = Request::builder()
        .uri(format!("/api/slo/slos/{id}"))
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, get_req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["name"], "find-me");
    assert_eq!(json["id"], id);
}

// ── DELETE /api/slo/slos/:id ─────────────────────────────────────────────────

#[tokio::test]
async fn test_delete_slo_not_found() {
    let state = new_state();
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/slo/slos/00000000-0000-0000-0000-000000000000")
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_slo_success() {
    let state = new_state();
    let body = serde_json::json!({
        "name": "delete-me", "description": "d", "target_percentage": 99.9,
        "window_days": 30, "metric_type": "error_rate"
    });
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/slo/slos")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let (_, created) = call(state.clone(), create_req).await;
    let id = created["id"].as_str().unwrap().to_string();

    let del_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/slo/slos/{id}"))
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(state.clone(), del_req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify gone
    let get_req = Request::builder()
        .uri(format!("/api/slo/slos/{id}"))
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(state, get_req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── GET /api/slo/slos/:id/budget ─────────────────────────────────────────────

#[tokio::test]
async fn test_get_budget_not_found() {
    let state = new_state();
    let req = Request::builder()
        .uri("/api/slo/slos/00000000-0000-0000-0000-000000000000/budget")
        .body(Body::empty())
        .unwrap();
    let (status, _) = call(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_budget_after_create() {
    let state = new_state();
    let body = serde_json::json!({
        "name": "budget-slo", "description": "d", "target_percentage": 99.9,
        "window_days": 30, "metric_type": "availability"
    });
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/slo/slos")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let (_, created) = call(state.clone(), create_req).await;
    let id = created["id"].as_str().unwrap().to_string();

    let req = Request::builder()
        .uri(format!("/api/slo/slos/{id}/budget"))
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert!(json["total_minutes"].is_number());
    assert!(json["allowed_bad_minutes"].is_number());
    assert!(json["is_breached"].is_boolean());
    assert!(!json["is_breached"].as_bool().unwrap());
}

// ── GET /api/slo/stats ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_stats_empty() {
    let state = new_state();
    let req = Request::builder()
        .uri("/api/slo/stats")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn test_stats_after_create() {
    let state = new_state();
    let body = serde_json::json!({
        "name": "stat-slo", "description": "d", "target_percentage": 99.9,
        "window_days": 30, "metric_type": "availability"
    });
    let create_req = Request::builder()
        .method("POST")
        .uri("/api/slo/slos")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    call(state.clone(), create_req).await;

    let req = Request::builder()
        .uri("/api/slo/stats")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 1);
}
