// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for DevLake HTTP routes — written BEFORE implementation (TDD).

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cave_devlake::{router, State};
use std::sync::Arc;
use tower::ServiceExt;

fn make_app() -> axum::Router {
    let state = Arc::new(State::default());
    router(state)
}

#[tokio::test]
async fn health_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_deployments_empty_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/deployments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_pipelines_empty_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/pipelines")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_incidents_empty_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/incidents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn dora_report_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/dora")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn dora_report_has_expected_fields() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/dora")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json.get("deployment_frequency_per_day").is_some());
    assert!(json.get("deployment_frequency_rating").is_some());
    assert!(json.get("change_failure_rate_pct").is_some());
    assert!(json.get("overall_rating").is_some());
}

#[tokio::test]
async fn list_prs_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/prs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_commits_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/commits")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_issues_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/issues")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn list_sprints_returns_ok() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/sprints")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn get_deployment_not_found_returns_404() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/deployments/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_pipeline_not_found_returns_404() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/pipelines/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_incident_not_found_returns_404() {
    let app = make_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/devlake/incidents/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
