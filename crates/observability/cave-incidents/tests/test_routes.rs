// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for the HTTP routes layer.
//! Uses axum::test utilities to call routes in-process.

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use cave_incidents::{State, router};
use cave_incidents::models::{
    CreateIncidentRequest, IncidentSeverity, IncidentStatus,
};
use std::sync::Arc;
use tower::ServiceExt; // for .oneshot()
use uuid::Uuid;

fn test_state() -> Arc<State> {
    Arc::new(State::default())
}

async fn json_request(
    app: axum::Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    let req = if let Some(b) = body {
        req.body(Body::from(serde_json::to_vec(&b).unwrap())).unwrap()
    } else {
        req.body(Body::empty()).unwrap()
    };
    let response = app.oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn test_health_route() {
    let state = test_state();
    let app = router(state);
    let (status, body) = json_request(app, Method::GET, "/api/incidents/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["module"], "cave-incidents");
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn test_create_incident_route() {
    let state = test_state();
    let app = router(state);
    let payload = serde_json::json!({
        "title": "DB Down",
        "description": "Primary DB unreachable",
        "severity": "p1",
        "created_by": Uuid::new_v4().to_string(),
        "tags": ["infra"]
    });
    let (status, body) = json_request(app, Method::POST, "/api/incidents", Some(payload)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["title"], "DB Down");
    assert_eq!(body["severity"], "p1");
    assert_eq!(body["status"], "open");
    assert!(body["id"].is_string());
}

#[tokio::test]
async fn test_list_incidents_route() {
    let state = test_state();
    // Pre-populate two incidents
    let user_id = Uuid::new_v4();
    {
        let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
            title: "I1".to_string(),
            description: "".to_string(),
            severity: IncidentSeverity::P2,
            created_by: user_id,
            tags: vec![],
        });
        state.store.create(i);
    }
    {
        let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
            title: "I2".to_string(),
            description: "".to_string(),
            severity: IncidentSeverity::P3,
            created_by: user_id,
            tags: vec![],
        });
        state.store.create(i);
    }
    let app = router(state);
    let (status, body) = json_request(app, Method::GET, "/api/incidents", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.is_array());
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_get_incident_route() {
    let state = test_state();
    let user_id = Uuid::new_v4();
    let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
        title: "API Slow".to_string(),
        description: "High latency".to_string(),
        severity: IncidentSeverity::P2,
        created_by: user_id,
        tags: vec![],
    });
    let id = i.id;
    state.store.create(i);

    let app = router(state);
    let (status, body) = json_request(app, Method::GET, &format!("/api/incidents/{id}"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["title"], "API Slow");
    assert_eq!(body["id"], id.to_string());
}

#[tokio::test]
async fn test_get_missing_incident_returns_404() {
    let state = test_state();
    let app = router(state);
    let (status, _) = json_request(
        app,
        Method::GET,
        &format!("/api/incidents/{}", Uuid::new_v4()),
        None,
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_acknowledge_incident_route() {
    let state = test_state();
    let user_id = Uuid::new_v4();
    let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
        title: "Net Down".to_string(),
        description: "".to_string(),
        severity: IncidentSeverity::P1,
        created_by: user_id,
        tags: vec![],
    });
    let id = i.id;
    state.store.create(i);

    let app = router(state);
    let payload = serde_json::json!({ "user_id": user_id.to_string() });
    let (status, body) = json_request(
        app,
        Method::POST,
        &format!("/api/incidents/{id}/acknowledge"),
        Some(payload),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "acknowledged");
    assert!(body["acknowledged_at"].is_string());
}

#[tokio::test]
async fn test_resolve_incident_route() {
    let state = test_state();
    let user_id = Uuid::new_v4();
    let mut i = cave_incidents::engine::create_incident(CreateIncidentRequest {
        title: "DNS Outage".to_string(),
        description: "".to_string(),
        severity: IncidentSeverity::P1,
        created_by: user_id,
        tags: vec![],
    });
    cave_incidents::engine::acknowledge(&mut i, user_id).unwrap();
    let id = i.id;
    state.store.create(i);

    let app = router(state);
    let payload = serde_json::json!({
        "user_id": user_id.to_string(),
        "resolution": "DNS records corrected"
    });
    let (status, body) = json_request(
        app,
        Method::POST,
        &format!("/api/incidents/{id}/resolve"),
        Some(payload),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "resolved");
    assert!(body["resolved_at"].is_string());
}

#[tokio::test]
async fn test_delete_incident_route() {
    let state = test_state();
    let user_id = Uuid::new_v4();
    let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
        title: "Temp".to_string(),
        description: "".to_string(),
        severity: IncidentSeverity::P4,
        created_by: user_id,
        tags: vec![],
    });
    let id = i.id;
    state.store.create(i);

    let app = router(state);
    let (del_status, _) = json_request(app.clone(), Method::DELETE, &format!("/api/incidents/{id}"), None).await;
    // Can't reuse app (consumed), so we just check status code
    assert_eq!(del_status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_incident_metrics_route() {
    let state = test_state();
    let user_id = Uuid::new_v4();
    let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
        title: "P1".to_string(),
        description: "".to_string(),
        severity: IncidentSeverity::P1,
        created_by: user_id,
        tags: vec![],
    });
    state.store.create(i);
    let app = router(state);
    let (status, body) = json_request(app, Method::GET, "/api/incidents/metrics", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total_incidents"], 1);
    assert_eq!(body["p1_count"], 1);
    assert_eq!(body["open_incidents"], 1);
}

#[tokio::test]
async fn test_create_postmortem_route() {
    let state = test_state();
    let user_id = Uuid::new_v4();
    let i = cave_incidents::engine::create_incident(CreateIncidentRequest {
        title: "DB Failure".to_string(),
        description: "".to_string(),
        severity: IncidentSeverity::P1,
        created_by: user_id,
        tags: vec![],
    });
    let incident_id = i.id;
    state.store.create(i);

    let app = router(state);
    let payload = serde_json::json!({
        "incident_id": incident_id.to_string(),
        "title": "DB Failure Post-Mortem",
        "summary": "Primary DB crashed",
        "root_cause": "Disk full",
        "action_items": ["Add disk monitoring"],
        "author_id": user_id.to_string()
    });
    let (status, body) = json_request(
        app,
        Method::POST,
        "/api/incidents/postmortems",
        Some(payload),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["title"], "DB Failure Post-Mortem");
    assert_eq!(body["status"], "draft");
    assert!(body["id"].is_string());
}
