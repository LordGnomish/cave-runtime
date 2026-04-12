//! HTTP routes for cave-policy.
//!
//! OPA REST API (v1):
//!   POST   /v1/data/{path}       — query with input
//!   GET    /v1/data/{path}       — query without input
//!   PUT    /v1/data/{path}       — create/overwrite document
//!   PATCH  /v1/data/{path}       — JSON Patch document
//!   DELETE /v1/data/{path}       — delete document
//!   GET    /v1/policies          — list all policies
//!   GET    /v1/policies/{id}     — get policy
//!   PUT    /v1/policies/{id}     — create/update policy
//!   DELETE /v1/policies/{id}     — delete policy
//!   POST   /v1/compile           — partial evaluation
//!   GET    /v1/query             — ad-hoc query (GET)
//!   POST   /v1/query             — ad-hoc query (POST)
//!   GET    /v1/health            — health check
//!   GET    /v1/status            — status (bundles, plugins)
//!
//! Kyverno API:
//!   GET    /api/kyverno/policies                — list ClusterPolicies
//!   POST   /api/kyverno/policies                — create ClusterPolicy
//!   GET    /api/kyverno/policies/{name}         — get ClusterPolicy
//!   PUT    /api/kyverno/policies/{name}         — update ClusterPolicy
//!   DELETE /api/kyverno/policies/{name}         — delete ClusterPolicy
//!   POST   /api/kyverno/evaluate                — evaluate policies against resource
//!   GET    /api/kyverno/reports                 — list PolicyReports
//!
//! K8s Admission Webhook:
//!   POST   /webhook/validate                    — ValidatingWebhook handler
//!   POST   /webhook/mutate                      — MutatingWebhook handler
//!
//! Cave module:
//!   GET    /api/policy/health                   — module health

use crate::models::*;
use crate::PolicyState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use std::sync::Arc;

pub fn create_router(state: Arc<PolicyState>) -> Router {
    Router::new()
        // ── Cave module health ──────────────────────────────────────────────
        .route("/api/policy/health", get(health))
        // ── OPA Data API ───────────────────────────────────────────────────
        .route("/v1/data", get(opa_get_data_root).post(opa_query_root).put(opa_put_data_root).patch(opa_patch_data_root))
        .route("/v1/data/*path", get(opa_get_data).post(opa_query_data).put(opa_put_data).patch(opa_patch_data).delete(opa_delete_data))
        // ── OPA Policy API ─────────────────────────────────────────────────
        .route("/v1/policies", get(opa_list_policies))
        .route("/v1/policies/:id", get(opa_get_policy).put(opa_put_policy).delete(opa_delete_policy))
        // ── OPA Compile API ────────────────────────────────────────────────
        .route("/v1/compile", post(opa_compile))
        // ── OPA Query API ──────────────────────────────────────────────────
        .route("/v1/query", get(opa_get_query).post(opa_post_query))
        // ── OPA Status & Health ────────────────────────────────────────────
        .route("/v1/health", get(opa_health))
        .route("/v1/status", get(opa_status))
        // ── Kyverno ClusterPolicy API ──────────────────────────────────────
        .route("/api/kyverno/policies", get(kyverno_list_policies).post(kyverno_create_policy))
        .route("/api/kyverno/policies/:name", get(kyverno_get_policy).put(kyverno_update_policy).delete(kyverno_delete_policy))
        .route("/api/kyverno/evaluate", post(kyverno_evaluate))
        .route("/api/kyverno/reports", get(kyverno_list_reports))
        // ── Admission Webhooks ─────────────────────────────────────────────
        .route("/webhook/validate", post(webhook_validate))
        .route("/webhook/mutate", post(webhook_mutate))
        // ── Decision Log ───────────────────────────────────────────────────
        .route("/api/policy/decisions", get(list_decisions))
        .with_state(state)
}

// ─── Module health ─────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-policy",
        "status": "ok",
        "upstream": ["OPA Gatekeeper", "OPAL", "Kyverno"],
        "upstream_tracked_version": {
            "opa": "0.69.x",
            "kyverno": "1.12.x"
        }
    }))
}

// ─── OPA Data API ─────────────────────────────────────────────────────────────

async fn opa_get_data_root(
    State(state): State<Arc<PolicyState>>,
    Query(params): Query<DataQueryParams>,
) -> Response {
    opa_query_impl(state, vec![], None, params).await
}

async fn opa_query_root(
    State(state): State<Arc<PolicyState>>,
    Query(params): Query<DataQueryParams>,
    body: Option<Json<DataQueryRequest>>,
) -> Response {
    let input = body.and_then(|b| b.input.clone());
    opa_query_impl(state, vec![], input, params).await
}

async fn opa_get_data(
    State(state): State<Arc<PolicyState>>,
    Path(path): Path<String>,
    Query(params): Query<DataQueryParams>,
) -> Response {
    let path_parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    opa_query_impl(state, path_parts, None, params).await
}

async fn opa_query_data(
    State(state): State<Arc<PolicyState>>,
    Path(path): Path<String>,
    Query(params): Query<DataQueryParams>,
    body: Option<Json<DataQueryRequest>>,
) -> Response {
    let path_parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    let input = body.and_then(|b| b.input.clone());
    opa_query_impl(state, path_parts, input, params).await
}

async fn opa_query_impl(
    state: Arc<PolicyState>,
    path: Vec<String>,
    input: Option<serde_json::Value>,
    _params: DataQueryParams,
) -> Response {
    let start = std::time::Instant::now();
    let input_val = input.unwrap_or(serde_json::Value::Null);

    let engine = state.rego.read().unwrap();
    let query_path = {
        let mut full = vec!["data".to_string()];
        full.extend(path.clone());
        full
    };

    let result = engine.query_path(&query_path, input_val.clone());

    let elapsed_ns = start.elapsed().as_nanos() as u64;

    // Record decision
    let path_str = format!("data/{}", path.join("/"));
    state.decision_log.record(
        &path_str,
        Some(&input_val),
        result.as_ref(),
        None,
        "http",
    );

    let resp = DataResponse {
        result,
        metrics: Some(serde_json::json!({
            "timer_rego_query_eval_ns": elapsed_ns
        })),
        provenance: None,
        explanation: None,
        decision_id: Some(uuid::Uuid::new_v4().to_string()),
        warning: None,
    };

    Json(resp).into_response()
}

async fn opa_put_data_root(
    State(state): State<Arc<PolicyState>>,
    Json(value): Json<serde_json::Value>,
) -> StatusCode {
    let mut engine = state.rego.write().unwrap();
    engine.replace_data(value);
    StatusCode::NO_CONTENT
}

async fn opa_put_data(
    State(state): State<Arc<PolicyState>>,
    Path(path): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> StatusCode {
    let path_parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    let mut engine = state.rego.write().unwrap();
    engine.set_data(&path_parts, value);
    StatusCode::NO_CONTENT
}

async fn opa_patch_data_root(
    State(state): State<Arc<PolicyState>>,
    Json(patches): Json<PatchDataRequest>,
) -> Response {
    let mut engine = state.rego.write().unwrap();
    match engine.patch_data(&[], &patches) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { code: "patch_error".into(), message: e.to_string(), errors: None }),
        ).into_response(),
    }
}

async fn opa_patch_data(
    State(state): State<Arc<PolicyState>>,
    Path(path): Path<String>,
    Json(patches): Json<PatchDataRequest>,
) -> Response {
    let path_parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    let mut engine = state.rego.write().unwrap();
    match engine.patch_data(&path_parts, &patches) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { code: "patch_error".into(), message: e.to_string(), errors: None }),
        ).into_response(),
    }
}

async fn opa_delete_data(
    State(state): State<Arc<PolicyState>>,
    Path(path): Path<String>,
) -> StatusCode {
    let path_parts: Vec<String> = path.split('/').filter(|s| !s.is_empty()).map(String::from).collect();
    let mut engine = state.rego.write().unwrap();
    engine.set_data(&path_parts, serde_json::Value::Null);
    StatusCode::NO_CONTENT
}

// ─── OPA Policy API ───────────────────────────────────────────────────────────

async fn opa_list_policies(
    State(state): State<Arc<PolicyState>>,
) -> Json<ListPoliciesResponse> {
    let engine = state.rego.read().unwrap();
    let policies: Vec<StoredPolicy> = engine
        .module_ids()
        .iter()
        .map(|&id| StoredPolicy {
            id: id.to_string(),
            raw: String::new(), // raw not stored in memory; would come from DB
            ast: engine.module_ast(id),
        })
        .collect();
    Json(ListPoliciesResponse { result: policies })
}

async fn opa_get_policy(
    State(state): State<Arc<PolicyState>>,
    Path(id): Path<String>,
) -> Response {
    let engine = state.rego.read().unwrap();
    let ast = engine.module_ast(&id);
    if ast.is_some() {
        Json(GetPolicyResponse {
            result: StoredPolicy { id, raw: String::new(), ast },
        }).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: "policy_not_found".into(),
                message: format!("policy '{id}' not found"),
                errors: None,
            }),
        ).into_response()
    }
}

async fn opa_put_policy(
    State(state): State<Arc<PolicyState>>,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let mut engine = state.rego.write().unwrap();
    match engine.load_module(&id, &body) {
        Ok(pkg) => {
            let result = StoredPolicy {
                id: id.clone(),
                raw: body,
                ast: engine.module_ast(&id),
            };
            tracing::info!(target: "cave_policy", policy_id = id, package = pkg, "policy loaded");
            Json(PutPolicyResponse { result }).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                code: "policy_compile_error".into(),
                message: e.to_string(),
                errors: None,
            }),
        ).into_response(),
    }
}

async fn opa_delete_policy(
    State(state): State<Arc<PolicyState>>,
    Path(id): Path<String>,
) -> Response {
    let mut engine = state.rego.write().unwrap();
    let existed = engine.module_ids().contains(&id.as_str());
    engine.remove_module(&id);
    if existed {
        StatusCode::OK.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: "policy_not_found".into(),
                message: format!("policy '{id}' not found"),
                errors: None,
            }),
        ).into_response()
    }
}

// ─── OPA Compile API ──────────────────────────────────────────────────────────

async fn opa_compile(
    State(state): State<Arc<PolicyState>>,
    Json(req): Json<CompileRequest>,
) -> Response {
    let engine = state.rego.read().unwrap();
    let unknowns = req.unknowns.as_deref().unwrap_or(&[]);
    match engine.partial_eval(&req.query, req.input, unknowns) {
        Ok(partial) => Json(CompileResponse {
            result: CompileResult {
                queries: partial.queries,
                support: partial.support,
            },
            metrics: None,
        }).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { code: "compile_error".into(), message: e.to_string(), errors: None }),
        ).into_response(),
    }
}

// ─── OPA Query API ────────────────────────────────────────────────────────────

async fn opa_get_query(
    State(state): State<Arc<PolicyState>>,
    Query(params): Query<AdHocQueryParams>,
) -> Response {
    let engine = state.rego.read().unwrap();
    match engine.query_str(&params.q, serde_json::Value::Null) {
        Ok(bindings) => Json(QueryResponse { result: bindings, metrics: None, explanation: None }).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { code: "query_error".into(), message: e.to_string(), errors: None }),
        ).into_response(),
    }
}

async fn opa_post_query(
    State(state): State<Arc<PolicyState>>,
    Json(req): Json<AdHocQueryRequest>,
) -> Response {
    let engine = state.rego.read().unwrap();
    let input = req.input.unwrap_or_default();
    match engine.query_str(&req.query, input) {
        Ok(bindings) => Json(QueryResponse { result: bindings, metrics: None, explanation: None }).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { code: "query_error".into(), message: e.to_string(), errors: None }),
        ).into_response(),
    }
}

// ─── OPA Status & Health ──────────────────────────────────────────────────────

async fn opa_health(State(state): State<Arc<PolicyState>>) -> Json<HealthResponse> {
    let bundles = state.bundles.read().unwrap();
    Json(HealthResponse {
        status: "ok".into(),
        bundles: Some(bundles.all_statuses().clone()),
        plugins: None,
    })
}

async fn opa_status(State(state): State<Arc<PolicyState>>) -> Json<StatusResponse> {
    let bundles = state.bundles.read().unwrap();
    let mut plugin_status = std::collections::HashMap::new();
    plugin_status.insert("kyverno".into(), PluginStatus {
        state: "OK".into(),
        message: None,
    });
    plugin_status.insert("decision_logger".into(), PluginStatus {
        state: if state.decision_log.is_enabled() { "OK" } else { "NOT_READY" }.into(),
        message: None,
    });
    Json(StatusResponse {
        result: OpaStatus {
            labels: std::collections::HashMap::new(),
            bundles: bundles.all_statuses().clone(),
            plugins: plugin_status,
            metrics: None,
        },
    })
}

// ─── Kyverno API ──────────────────────────────────────────────────────────────

async fn kyverno_list_policies(
    State(state): State<Arc<PolicyState>>,
) -> Json<serde_json::Value> {
    let engine = state.kyverno.read().unwrap();
    let policies: Vec<&crate::kyverno::models::ClusterPolicy> = engine.list_cluster_policies();
    Json(serde_json::json!({ "items": policies }))
}

async fn kyverno_create_policy(
    State(state): State<Arc<PolicyState>>,
    Json(policy): Json<crate::kyverno::models::ClusterPolicy>,
) -> Response {
    let mut engine = state.kyverno.write().unwrap();
    engine.add_cluster_policy(policy.clone());
    tracing::info!(target: "cave_policy", policy = policy.metadata.name, "ClusterPolicy created");
    (StatusCode::CREATED, Json(policy)).into_response()
}

async fn kyverno_get_policy(
    State(state): State<Arc<PolicyState>>,
    Path(name): Path<String>,
) -> Response {
    let engine = state.kyverno.read().unwrap();
    match engine.get_cluster_policy(&name) {
        Some(p) => Json(p.clone()).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                code: "not_found".into(),
                message: format!("ClusterPolicy '{name}' not found"),
                errors: None,
            }),
        ).into_response(),
    }
}

async fn kyverno_update_policy(
    State(state): State<Arc<PolicyState>>,
    Path(_name): Path<String>,
    Json(policy): Json<crate::kyverno::models::ClusterPolicy>,
) -> Json<crate::kyverno::models::ClusterPolicy> {
    let mut engine = state.kyverno.write().unwrap();
    engine.add_cluster_policy(policy.clone());
    Json(policy)
}

async fn kyverno_delete_policy(
    State(state): State<Arc<PolicyState>>,
    Path(name): Path<String>,
) -> StatusCode {
    let mut engine = state.kyverno.write().unwrap();
    engine.remove_cluster_policy(&name);
    StatusCode::NO_CONTENT
}

#[derive(serde::Deserialize)]
struct KyvernoEvalRequest {
    resource: serde_json::Value,
    namespace: Option<String>,
    operation: Option<String>,
}

async fn kyverno_evaluate(
    State(state): State<Arc<PolicyState>>,
    Json(req): Json<KyvernoEvalRequest>,
) -> Json<crate::kyverno::models::PolicyEvalResult> {
    let engine = state.kyverno.read().unwrap();
    let operation = req.operation.as_deref().unwrap_or("CREATE");
    let result = engine.evaluate(&req.resource, req.namespace.as_deref(), operation, None);
    Json(result)
}

async fn kyverno_list_reports(
    State(state): State<Arc<PolicyState>>,
) -> Json<serde_json::Value> {
    let engine = state.kyverno.read().unwrap();
    let report = engine.generate_report(None);
    Json(serde_json::json!({ "items": [report] }))
}

// ─── Admission Webhooks ───────────────────────────────────────────────────────

async fn webhook_validate(
    State(state): State<Arc<PolicyState>>,
    Json(review): Json<crate::admission::AdmissionReview>,
) -> Response {
    let opa = state.rego.read().unwrap();
    let kyverno = state.kyverno.read().unwrap();
    let webhook = state.webhook.read().unwrap();

    match webhook.handle(&review, &*opa, &*kyverno) {
        Ok(response_review) => Json(response_review).into_response(),
        Err(e) => {
            tracing::error!(target: "admission", error = e.to_string(), "webhook error");
            if state.fail_open {
                // Fail-open: allow the request
                let uid = review.request.as_ref().map(|r| r.uid.clone()).unwrap_or_default();
                Json(crate::admission::AdmissionReview::new_response(
                    uid.clone(),
                    crate::admission::AdmissionResponse::allow(uid),
                )).into_response()
            } else {
                // Fail-closed: deny the request
                let uid = review.request.as_ref().map(|r| r.uid.clone()).unwrap_or_default();
                Json(crate::admission::AdmissionReview::new_response(
                    uid.clone(),
                    crate::admission::AdmissionResponse::deny(
                        uid,
                        format!("internal error: {e}"),
                        500,
                    ),
                )).into_response()
            }
        }
    }
}

async fn webhook_mutate(
    State(state): State<Arc<PolicyState>>,
    Json(review): Json<crate::admission::AdmissionReview>,
) -> Response {
    // Mutating webhook: same evaluation, but mutations are returned in patch
    webhook_validate(State(state), Json(review)).await
}

// ─── Decision Log ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct DecisionListParams {
    limit: Option<usize>,
}

async fn list_decisions(
    State(state): State<Arc<PolicyState>>,
    Query(params): Query<DecisionListParams>,
) -> Json<serde_json::Value> {
    let limit = params.limit.unwrap_or(100);
    let decisions = state.decision_log.recent_decisions(limit);
    Json(serde_json::json!({ "result": decisions }))
}
