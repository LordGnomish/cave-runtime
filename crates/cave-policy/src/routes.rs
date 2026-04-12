//! HTTP routes — OPA-compatible admin API + admission webhook.
//!
//! Endpoints:
//!   GET  /v1/policies                 — list all policies
//!   PUT  /v1/policies/{id}            — create / replace policy
//!   GET  /v1/policies/{id}            — get policy
//!   DELETE /v1/policies/{id}          — delete policy
//!   POST /v1/data                     — query rule (evaluate)
//!   POST /v1/compile                  — compile a policy (check only)
//!   GET  /v1/config                   — show agent config
//!   GET  /v1/status                   — agent health + bundle status
//!   POST /v1/admission                — Kubernetes admission webhook

use crate::{
    admission::{self, AdmissionReview},
    engine,
    State,
};
use axum::{
    extract::{Path, State as AxumState},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

pub fn create_router(state: Arc<State>) -> Router {
    Router::new()
        // legacy health endpoint
        .route("/api/policy/health", get(health))
        // OPA-compatible admin API
        .route("/v1/status",             get(status))
        .route("/v1/config",             get(config))
        .route("/v1/policies",           get(list_policies))
        .route("/v1/policies/{id}",      put(put_policy))
        .route("/v1/policies/{id}",      get(get_policy))
        .route("/v1/policies/{id}",      delete(delete_policy))
        .route("/v1/data",               post(query_data))
        .route("/v1/compile",            post(compile_policy))
        // Kubernetes admission webhook
        .route("/v1/admission",          post(admission_webhook))
        .with_state(state)
}

// ── Health / Status ───────────────────────────────────────────────────────────

async fn health() -> Json<Value> {
    Json(json!({
        "module": "cave-policy",
        "status": "ok",
        "upstream": "OPA Gatekeeper + OPAL"
    }))
}

async fn status(AxumState(state): AxumState<Arc<State>>) -> Json<Value> {
    let store = state.bundles.read().await;
    let active = store.active_bundles().len();
    let total  = store.list().len();
    Json(json!({
        "result": {
            "state": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "bundles": {
                "active": active,
                "total":  total,
            },
            "decision_log": {
                "entries": state.decision_log.len(),
            }
        }
    }))
}

async fn config() -> Json<Value> {
    Json(json!({
        "result": {
            "default_decision": "/http/authz/allow",
            "default_authorization_decision": "/system/authz/allow",
            "plugins": {
                "decision_logs": { "console": false },
                "status": { "console": true }
            }
        }
    }))
}

// ── Policy CRUD ───────────────────────────────────────────────────────────────

async fn list_policies(AxumState(state): AxumState<Arc<State>>) -> Json<Value> {
    let policies = state.policies.read().await;
    let list: Vec<Value> = policies
        .iter()
        .map(|(id, src)| json!({ "id": id, "raw": src }))
        .collect();
    Json(json!({ "result": list }))
}

#[derive(Deserialize)]
struct PutPolicyBody {
    raw: String,
}

async fn put_policy(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<String>,
    Json(body): Json<PutPolicyBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // Try to compile first
    if let Err(e) = engine::compile(&body.raw) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e })),
        ));
    }
    state.policies.write().await.insert(id.clone(), body.raw);
    Ok(Json(json!({ "result": null })))
}

async fn get_policy(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let policies = state.policies.read().await;
    match policies.get(&id) {
        Some(raw) => Ok(Json(json!({ "result": { "id": id, "raw": raw } }))),
        None => Err((StatusCode::NOT_FOUND, Json(json!({ "error": "not found" })))),
    }
}

async fn delete_policy(
    AxumState(state): AxumState<Arc<State>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut policies = state.policies.write().await;
    if policies.remove(&id).is_some() {
        Ok(Json(json!({ "result": null })))
    } else {
        Err((StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))))
    }
}

// ── Data query ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct QueryBody {
    input: Value,
    #[serde(default)]
    data: Value,
    #[serde(default)]
    policy_id: Option<String>,
}

async fn query_data(
    AxumState(state): AxumState<Arc<State>>,
    Json(body): Json<QueryBody>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let policies = state.policies.read().await;

    // Pick target policy
    let src = if let Some(id) = &body.policy_id {
        policies.get(id.as_str()).ok_or_else(|| {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "policy not found" })))
        })?.clone()
    } else {
        // Use first available
        match policies.values().next() {
            Some(s) => s.clone(),
            None => return Ok(Json(json!({ "result": {} }))),
        }
    };
    drop(policies);

    let policy = engine::compile(&src).map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(json!({ "error": e })))
    })?;

    let start = std::time::Instant::now();
    let result = engine::evaluate(&policy, &body.input, &body.data);
    let elapsed_us = start.elapsed().as_micros() as u64;

    let result_val = serde_json::to_value(&result).unwrap_or(Value::Null);

    state.decision_log.record(
        body.policy_id.as_deref().unwrap_or("default"),
        "/v1/data",
        body.input.clone(),
        result_val.clone(),
        elapsed_us,
    );

    Ok(Json(json!({ "result": result_val })))
}

// ── Compile ───────────────────────────────────────────────────────────────────

async fn compile_policy(Json(body): Json<Value>) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    // If a "raw" field is present, try to compile it as Rego
    if let Some(raw) = body.get("raw").and_then(|v| v.as_str()) {
        return match engine::compile(raw) {
            Ok(_) => Ok(Json(json!({ "result": { "queries": [] } }))),
            Err(e) => Err((StatusCode::BAD_REQUEST, Json(json!({ "error": e })))),
        };
    }
    // Otherwise echo back (OPA compile endpoint passes partial eval queries)
    Ok(Json(json!({ "result": { "queries": [] } })))
}

// ── Admission webhook ─────────────────────────────────────────────────────────

async fn admission_webhook(
    AxumState(state): AxumState<Arc<State>>,
    Json(review): Json<AdmissionReview>,
) -> Json<AdmissionReview> {
    // Load admission policy (if configured)
    let policies = state.policies.read().await;
    let admission_src = policies.get("admission").cloned();
    drop(policies);

    let data = Value::Object(Default::default());

    let response = if let Some(src) = admission_src {
        match engine::compile(&src) {
            Ok(policy) => admission::evaluate_admission(&review, &policy, &data),
            Err(_) => AdmissionReview::new_response(
                review.request.as_ref().map(|r| r.uid.clone()).unwrap_or_default(),
                false,
                Some("admission policy compile error".into()),
            ),
        }
    } else {
        // No policy → deny-by-default
        AdmissionReview::new_response(
            review.request.as_ref().map(|r| r.uid.clone()).unwrap_or_default(),
            false,
            Some("no admission policy configured".into()),
        )
    };

    Json(response)
}
