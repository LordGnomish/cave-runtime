// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP wiring test for the query-scheduler fair-share preview endpoint.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt; // for .oneshot()

fn app() -> axum::Router {
    cave_logs::router(cave_logs::default_state())
}

async fn post_json(path: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn scheduler_preview_round_robins_fairly() {
    let (status, v) = post_json(
        "/loki/api/v1/scheduler/preview",
        json!({
            "tenants": [
                {"tenant": "a", "count": 2},
                {"tenant": "b", "count": 2}
            ],
            "consumers": ["q0"]
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(v["enqueued"], 4);
    assert_eq!(v["served"], 4);
    let order: Vec<String> = v["dispatch_order"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    // Fair round-robin: no tenant served twice in a row.
    assert_eq!(order, vec!["a", "b", "a", "b"]);
}

#[tokio::test]
async fn scheduler_preview_reports_shuffle_shards() {
    let (status, v) = post_json(
        "/loki/api/v1/scheduler/preview",
        json!({
            "tenants": [{"tenant": "tenant-a", "count": 1}],
            "consumers": ["q0", "q1", "q2", "q3"],
            "max_consumers": 2
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    // tenant-a should be pinned to exactly 2 of the 4 queriers.
    let shard = v["shuffle_shards"]["tenant-a"].as_array().unwrap();
    assert_eq!(shard.len(), 2);
}
