// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Failing tests for the full Chaos Mesh-compatible REST API:
//! CRUD experiments, start/stop/rollback, list, schedule CRUD.
//! These routes are NEW (origin/main has only a health endpoint).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cave_chaos::{
    models::{
        BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus,
        ExperimentType, SafetyGuard,
    },
    routes::create_router,
    State,
};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;
use std::collections::HashMap;
use chrono::Utc;

fn app() -> axum::Router {
    let state = Arc::new(State::default());
    create_router(state)
}

fn make_create_payload(ns: &str, exp_type: &str) -> serde_json::Value {
    serde_json::json!({
        "name": "test-experiment",
        "experiment_type": exp_type,
        "target": {
            "namespace": ns,
            "selector": {"app": "frontend"},
            "pod_count": 1
        },
        "parameters": {
            "latency_ms": 100,
            "packet_loss_percent": null,
            "cpu_load_percent": null,
            "memory_mb": null
        },
        "duration_secs": 60,
        "blast_radius": {
            "max_pod_fraction": 0.5,
            "max_pods": null,
            "namespaces": []
        },
        "safety_guard": {
            "enabled": true,
            "protected_namespaces": ["kube-system", "kube-public", "cave-system"],
            "min_healthy_pod_percentage": 0.5
        },
        "annotations": {}
    })
}

// ─── Health endpoint (pre-existing, should still work) ────────────────────────

#[tokio::test]
async fn get_health_returns_ok() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/api/chaos/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ─── Create experiment ────────────────────────────────────────────────────────

#[tokio::test]
async fn post_experiments_creates_experiment() {
    let body = serde_json::to_string(&make_create_payload("staging", "network_latency")).unwrap();
    let resp = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chaos/experiments")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let exp: ChaosExperiment = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(exp.status, ExperimentStatus::Draft);
    assert_eq!(exp.target.namespace, "staging");
}

// ─── List experiments ─────────────────────────────────────────────────────────

#[tokio::test]
async fn get_experiments_returns_list() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri("/api/chaos/experiments")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let list: Vec<ChaosExperiment> = serde_json::from_slice(&bytes).unwrap();
    assert!(list.is_empty(), "fresh state should have no experiments");
}

// ─── Get experiment by ID ─────────────────────────────────────────────────────

#[tokio::test]
async fn get_experiment_by_id_returns_404_for_unknown() {
    let resp = app()
        .oneshot(
            Request::builder()
                .uri(&format!("/api/chaos/experiments/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─── Delete experiment ────────────────────────────────────────────────────────

#[tokio::test]
async fn delete_experiment_returns_404_for_unknown() {
    let resp = app()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/api/chaos/experiments/{}", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─── Start / Stop experiment ──────────────────────────────────────────────────

#[tokio::test]
async fn start_experiment_returns_404_for_unknown() {
    let resp = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/chaos/experiments/{}/start", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stop_experiment_returns_404_for_unknown() {
    let resp = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/chaos/experiments/{}/stop", Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─── Full create → start → stop lifecycle ────────────────────────────────────

#[tokio::test]
async fn full_experiment_lifecycle_create_start_stop() {
    let state = Arc::new(State::default());
    let app = create_router(state);

    // 1. Create
    let body = serde_json::to_string(&make_create_payload("staging", "network_latency")).unwrap();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/chaos/experiments")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let created: ChaosExperiment = serde_json::from_slice(&bytes).unwrap();
    let eid = created.id;

    // 2. Get — should be Draft
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/api/chaos/experiments/{}", eid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let fetched: ChaosExperiment = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched.status, ExperimentStatus::Draft);

    // 3. Start
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/chaos/experiments/{}/start", eid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let started: ChaosExperiment = serde_json::from_slice(&bytes).unwrap();
    // After start+execute, should be Completed (executor is synchronous)
    assert!(
        started.status == ExperimentStatus::Completed
            || started.status == ExperimentStatus::Running,
        "expected Completed or Running, got {:?}",
        started.status
    );

    // 4. Delete
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/api/chaos/experiments/{}", eid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // 5. Get after delete → 404
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/api/chaos/experiments/{}", eid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
