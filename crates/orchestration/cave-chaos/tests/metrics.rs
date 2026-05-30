// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Strict-TDD: Prometheus metrics exporter. Renders chaos experiment counters in
//! Prometheus text exposition format and mounts a /api/chaos/metrics endpoint.

use cave_chaos::metrics::render_prometheus;
use cave_chaos::models::{
    BlastRadius, ChaosExperiment, ChaosTarget, ExperimentParams, ExperimentStatus, ExperimentType,
    SafetyGuard,
};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

fn exp(status: ExperimentStatus, ns: &str) -> ChaosExperiment {
    ChaosExperiment {
        id: Uuid::new_v4(),
        name: "m".to_string(),
        experiment_type: ExperimentType::PodKill,
        target: ChaosTarget { namespace: ns.to_string(), selector: HashMap::new(), pod_count: None },
        parameters: ExperimentParams { latency_ms: None, packet_loss_percent: None, cpu_load_percent: None, memory_mb: None },
        status,
        created_at: Utc::now(),
        started_at: None,
        ended_at: None,
        duration_secs: 30,
        blast_radius: BlastRadius::default(),
        safety_guard: SafetyGuard::default(),
        result: None,
        annotations: HashMap::new(),
    }
}

#[test]
fn test_render_has_type_lines() {
    let out = render_prometheus(&[]);
    assert!(out.contains("# TYPE chaos_experiments_total gauge"), "got:\n{out}");
    assert!(out.contains("# TYPE chaos_experiments_by_status gauge"), "got:\n{out}");
    assert!(out.contains("# TYPE chaos_experiments_active gauge"), "got:\n{out}");
}

#[test]
fn test_render_empty_is_all_zero() {
    let out = render_prometheus(&[]);
    assert!(out.contains("chaos_experiments_total 0"), "got:\n{out}");
    assert!(out.contains("chaos_experiments_active 0"), "got:\n{out}");
}

#[test]
fn test_render_total_count() {
    let exps = vec![
        exp(ExperimentStatus::Running, "staging"),
        exp(ExperimentStatus::Completed, "staging"),
        exp(ExperimentStatus::Draft, "staging"),
    ];
    let out = render_prometheus(&exps);
    assert!(out.contains("chaos_experiments_total 3"), "got:\n{out}");
}

#[test]
fn test_render_by_status_labels() {
    let exps = vec![
        exp(ExperimentStatus::Running, "staging"),
        exp(ExperimentStatus::Running, "staging"),
        exp(ExperimentStatus::Failed, "staging"),
    ];
    let out = render_prometheus(&exps);
    assert!(out.contains("chaos_experiments_by_status{status=\"running\"} 2"), "got:\n{out}");
    assert!(out.contains("chaos_experiments_by_status{status=\"failed\"} 1"), "got:\n{out}");
    assert!(out.contains("chaos_experiments_by_status{status=\"draft\"} 0"), "got:\n{out}");
}

#[test]
fn test_render_active_equals_running() {
    let exps = vec![
        exp(ExperimentStatus::Running, "staging"),
        exp(ExperimentStatus::Running, "staging"),
        exp(ExperimentStatus::Completed, "staging"),
    ];
    let out = render_prometheus(&exps);
    assert!(out.contains("chaos_experiments_active 2"), "got:\n{out}");
}

#[test]
fn test_render_high_risk_counts_production() {
    let exps = vec![
        exp(ExperimentStatus::Running, "production"),
        exp(ExperimentStatus::Running, "prod"),
        exp(ExperimentStatus::Running, "staging"),
    ];
    let out = render_prometheus(&exps);
    assert!(out.contains("chaos_experiments_high_risk 2"), "got:\n{out}");
}

#[tokio::test]
async fn test_metrics_endpoint_mounted() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    let state = std::sync::Arc::new(cave_chaos::State::default());
    let app = cave_chaos::router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/chaos/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
