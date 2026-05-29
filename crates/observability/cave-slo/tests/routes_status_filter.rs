// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for GET /api/slo/slos?status=<status> query parameter filtering.
//! Written FIRST per TDD — will fail until the route handler supports
//! the optional `status` query parameter.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cave_slo::{State, router};
use cave_slo::models::{MetricType, SloStatus};
use cave_slo::store::SloStore;
use serde_json::Value;
use std::sync::Arc;

fn new_state() -> Arc<State> {
    State::new()
}

async fn call(state: Arc<State>, req: Request<Body>) -> (StatusCode, Value) {
    use tower::ServiceExt;
    let app = router(state);
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
    (status, json)
}

/// Seed the store with SLOs of mixed statuses via the store directly.
fn seed_mixed_state() -> Arc<State> {
    use cave_slo::models::SLO;
    use chrono::Utc;
    use uuid::Uuid;

    let state = State::new();
    let slos = vec![
        SLO {
            id: Uuid::new_v4(), name: "ok-slo-1".into(), description: "d".into(),
            target_percentage: 99.9, window_days: 30, metric_type: MetricType::Availability,
            created_at: Utc::now(), current_sli: 99.95, status: SloStatus::Ok,
        },
        SLO {
            id: Uuid::new_v4(), name: "ok-slo-2".into(), description: "d".into(),
            target_percentage: 99.0, window_days: 7, metric_type: MetricType::Latency,
            created_at: Utc::now(), current_sli: 99.2, status: SloStatus::Ok,
        },
        SLO {
            id: Uuid::new_v4(), name: "at-risk-slo".into(), description: "d".into(),
            target_percentage: 99.9, window_days: 30, metric_type: MetricType::ErrorRate,
            created_at: Utc::now(), current_sli: 98.0, status: SloStatus::AtRisk,
        },
        SLO {
            id: Uuid::new_v4(), name: "breached-slo".into(), description: "d".into(),
            target_percentage: 99.9, window_days: 30, metric_type: MetricType::Availability,
            created_at: Utc::now(), current_sli: 90.0, status: SloStatus::Breached,
        },
    ];
    for slo in slos {
        state.store.insert(slo);
    }
    state
}

// ── GET /api/slo/slos?status=ok ───────────────────────────────────────────

#[tokio::test]
async fn test_list_slos_filter_by_status_ok() {
    let state = seed_mixed_state();
    let req = Request::builder()
        .uri("/api/slo/slos?status=ok")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let arr = json.as_array().expect("expected array");
    assert_eq!(arr.len(), 2, "expected 2 ok SLOs, got {}", arr.len());
    for item in arr {
        assert_eq!(item["status"], "ok", "item status={}", item["status"]);
    }
}

// ── GET /api/slo/slos?status=breached ─────────────────────────────────────

#[tokio::test]
async fn test_list_slos_filter_by_status_breached() {
    let state = seed_mixed_state();
    let req = Request::builder()
        .uri("/api/slo/slos?status=breached")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let arr = json.as_array().expect("expected array");
    assert_eq!(arr.len(), 1, "expected 1 breached SLO, got {}", arr.len());
    assert_eq!(arr[0]["name"], "breached-slo");
}

// ── GET /api/slo/slos?status=at_risk ──────────────────────────────────────

#[tokio::test]
async fn test_list_slos_filter_by_status_at_risk() {
    let state = seed_mixed_state();
    let req = Request::builder()
        .uri("/api/slo/slos?status=at_risk")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK);
    let arr = json.as_array().expect("expected array");
    assert_eq!(arr.len(), 1, "expected 1 at_risk SLO, got {}", arr.len());
    assert_eq!(arr[0]["name"], "at-risk-slo");
}

// ── GET /api/slo/slos (no filter) should still list all ──────────────────

#[tokio::test]
async fn test_list_slos_no_filter_returns_all() {
    let state = seed_mixed_state();
    let req = Request::builder()
        .uri("/api/slo/slos")
        .body(Body::empty())
        .unwrap();
    let (status, json) = call(state, req).await;
    assert_eq!(status, StatusCode::OK);
    let arr = json.as_array().expect("expected array");
    assert_eq!(arr.len(), 4, "expected all 4 SLOs, got {}", arr.len());
}

// ── GET /api/slo/slos?status=unknown_value should return 400 ─────────────

#[tokio::test]
async fn test_list_slos_filter_invalid_status_returns_400() {
    let state = seed_mixed_state();
    let req = Request::builder()
        .uri("/api/slo/slos?status=bogus_value")
        .body(Body::empty())
        .unwrap();
    let (status, _json) = call(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "invalid status should return 400");
}
