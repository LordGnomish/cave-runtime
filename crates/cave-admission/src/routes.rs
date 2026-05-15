// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-admission.

use crate::{
    engine::{evaluate_all_policies},
    models::{AdmissionResult, Operation, Policy, PolicyReport, Resource, Violation},
    AdmissionState,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<AdmissionState>) -> Router {
    Router::new()
        // Policies CRUD
        .route("/api/v1/policies", get(list_policies).post(create_policy))
        .route(
            "/api/v1/policies/{id}",
            get(get_policy).put(update_policy).delete(delete_policy),
        )
        // Admission review
        .route("/api/v1/admission/review", post(admission_review))
        // Reports
        .route("/api/v1/reports", get(generate_report))
        // Violations
        .route("/api/v1/violations", get(list_violations))
        // Health
        .route("/api/v1/admission/health", get(health))
        .with_state(state)
}

// ── Policies ─────────────────────────────────────────────────────────────────

async fn list_policies(State(state): State<Arc<AdmissionState>>) -> Json<Vec<Policy>> {
    Json(state.policies.read().await.clone())
}

async fn create_policy(
    State(state): State<Arc<AdmissionState>>,
    Json(policy): Json<Policy>,
) -> Json<Policy> {
    state.policies.write().await.push(policy.clone());
    Json(policy)
}

async fn get_policy(
    State(state): State<Arc<AdmissionState>>,
    Path(id): Path<Uuid>,
) -> Json<Option<Policy>> {
    Json(
        state
            .policies
            .read()
            .await
            .iter()
            .find(|p| p.id == id)
            .cloned(),
    )
}

async fn update_policy(
    State(state): State<Arc<AdmissionState>>,
    Path(id): Path<Uuid>,
    Json(updated): Json<Policy>,
) -> Json<Option<Policy>> {
    let mut policies = state.policies.write().await;
    if let Some(policy) = policies.iter_mut().find(|p| p.id == id) {
        *policy = updated.clone();
        Json(Some(updated))
    } else {
        Json(None)
    }
}

async fn delete_policy(
    State(state): State<Arc<AdmissionState>>,
    Path(id): Path<Uuid>,
) -> Json<bool> {
    let mut policies = state.policies.write().await;
    let before = policies.len();
    policies.retain(|p| p.id != id);
    Json(policies.len() < before)
}

// ── Admission review ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AdmissionRequest {
    pub resource: Resource,
    pub operation: Operation,
}

#[derive(Serialize)]
pub struct AdmissionResponse {
    pub result: AdmissionResult,
}

async fn admission_review(
    State(state): State<Arc<AdmissionState>>,
    Json(req): Json<AdmissionRequest>,
) -> Json<AdmissionResponse> {
    let result = {
        let policies = state.policies.read().await;
        evaluate_all_policies(&policies, &req.resource, req.operation)
    };

    if !result.violations.is_empty() {
        state
            .violations
            .write()
            .await
            .extend(result.violations.clone());
    }

    Json(AdmissionResponse { result })
}

// ── Reports ───────────────────────────────────────────────────────────────────

async fn generate_report(State(state): State<Arc<AdmissionState>>) -> Json<PolicyReport> {
    let violations = state.violations.read().await;
    let policies = state.policies.read().await;

    let mut violations_by_policy: HashMap<String, usize> = HashMap::new();
    for v in violations.iter() {
        *violations_by_policy.entry(v.policy_name.clone()).or_insert(0) += 1;
    }

    let failing: Vec<String> = violations_by_policy.keys().cloned().collect();
    let passing: Vec<String> = policies
        .iter()
        .filter(|p| !failing.contains(&p.name))
        .map(|p| p.name.clone())
        .collect();

    Json(PolicyReport {
        id: Uuid::new_v4(),
        generated_at: Utc::now(),
        total_resources_checked: 0,
        total_violations: violations.len(),
        violations_by_policy,
        passing_policies: passing,
        failing_policies: failing,
    })
}

// ── Violations ────────────────────────────────────────────────────────────────

async fn list_violations(State(state): State<Arc<AdmissionState>>) -> Json<Vec<Violation>> {
    Json(state.violations.read().await.clone())
}

// ── Health ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    module: &'static str,
    status: &'static str,
    upstream: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        module: "cave-admission",
        status: "ok",
        upstream: "Kyverno + Gatekeeper",
    })
}
