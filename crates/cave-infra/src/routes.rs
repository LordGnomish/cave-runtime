//! HTTP routes for cave-infra.

use crate::executor;
use crate::intent;
use crate::models::{ExecutionPlan, McpProvider};
use crate::planner;
use crate::state::StateSnapshot;
use crate::InfraModuleState;
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<InfraModuleState>) -> Router {
    Router::new()
        .route("/api/v1/infra/intent", post(submit_intent))
        .route("/api/v1/infra/plan", post(generate_plan))
        .route("/api/v1/infra/apply", post(apply_plan))
        .route("/api/v1/infra/destroy", post(destroy_resources))
        .route("/api/v1/infra/state", get(get_state))
        .route("/api/v1/infra/drift", get(detect_drift))
        .route(
            "/api/v1/infra/providers",
            get(list_providers).post(register_provider),
        )
        .route("/api/v1/infra/history", get(state_history))
        .route("/api/v1/infra/import", post(import_resource))
        .route("/api/v1/infra/cost", post(estimate_cost))
        .route("/api/v1/infra/health", get(health))
        .with_state(state)
}

// ── Request / Response DTOs ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct IntentRequest {
    description: String,
    yaml: Option<String>,
}

#[derive(Deserialize)]
struct PlanRequest {
    description: String,
    yaml: Option<String>,
}

#[derive(Deserialize)]
struct ApplyRequest {
    plan_id: Uuid,
}

#[derive(Deserialize)]
struct DestroyRequest {
    resource_names: Vec<String>,
}

#[derive(Deserialize)]
struct RegisterProviderRequest {
    name: String,
    endpoint: String,
}

#[derive(Deserialize)]
struct ImportRequest {
    name: String,
    provider: String,
    resource_type: String,
    actual_id: String,
}

#[derive(Deserialize)]
struct CostRequest {
    description: String,
    yaml: Option<String>,
}

#[derive(Serialize)]
struct PlanSummary {
    id: Uuid,
    intent_id: Uuid,
    steps: usize,
    risk_score: u8,
    monthly_usd: f64,
    explanation: String,
    status: String,
}

impl From<&ExecutionPlan> for PlanSummary {
    fn from(p: &ExecutionPlan) -> Self {
        PlanSummary {
            id: p.id,
            intent_id: p.intent_id,
            steps: p.steps.len(),
            risk_score: p.risk_score,
            monthly_usd: p.cost_estimate.monthly_usd,
            explanation: p.explanation.clone(),
            status: format!("{:?}", p.status),
        }
    }
}

#[derive(Serialize)]
struct ProviderSummary {
    id: Uuid,
    name: String,
    endpoint: String,
    healthy: bool,
    tool_count: usize,
}

impl From<&McpProvider> for ProviderSummary {
    fn from(p: &McpProvider) -> Self {
        ProviderSummary {
            id: p.id,
            name: p.name.clone(),
            endpoint: p.endpoint.clone(),
            healthy: p.healthy,
            tool_count: p.capabilities.len(),
        }
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// Submit an infrastructure intent (natural language or YAML).
async fn submit_intent(
    State(state): State<Arc<InfraModuleState>>,
    Json(req): Json<IntentRequest>,
) -> Json<serde_json::Value> {
    match intent::parse_intent(&req.description, req.yaml.as_deref()) {
        Ok(infra_intent) => {
            let id = infra_intent.id;
            let constraints = infra_intent.constraints.clone();
            let resource_count = infra_intent.resources.len();
            state.intents.lock().await.push(infra_intent);
            Json(serde_json::json!({
                "intent_id": id,
                "resources_inferred": resource_count,
                "constraints": constraints,
            }))
        }
        Err(e) => Json(serde_json::json!({"error": e.to_string()})),
    }
}

/// Generate an execution plan from an intent.
async fn generate_plan(
    State(state): State<Arc<InfraModuleState>>,
    Json(req): Json<PlanRequest>,
) -> Json<serde_json::Value> {
    let infra_intent = match intent::parse_intent(&req.description, req.yaml.as_deref()) {
        Ok(i) => i,
        Err(e) => return Json(serde_json::json!({"error": e.to_string()})),
    };

    let registry = state.registry.lock().await;
    let store = state.store.lock().await;

    let plan = planner::generate_plan(&infra_intent, &store.state, &registry.providers);
    let summary = PlanSummary::from(&plan);
    drop(registry);
    drop(store);

    state.plans.lock().await.push(plan);
    Json(serde_json::json!(summary))
}

/// Apply an existing plan — execute it via MCP.
async fn apply_plan(
    State(state): State<Arc<InfraModuleState>>,
    Json(req): Json<ApplyRequest>,
) -> Json<serde_json::Value> {
    // Clone the plan so we can release the plans lock before acquiring other locks.
    let plan_clone = {
        let plans = state.plans.lock().await;
        plans.iter().find(|p| p.id == req.plan_id).cloned()
    };

    let Some(mut plan) = plan_clone else {
        return Json(serde_json::json!({"error": "plan not found"}));
    };

    let registry = state.registry.lock().await;
    let mut store = state.store.lock().await;
    let result = executor::execute_plan(&mut plan, &*registry, &mut *store).await;
    drop(registry);
    drop(store);

    // Persist updated plan status.
    let mut plans = state.plans.lock().await;
    if let Some(p) = plans.iter_mut().find(|p| p.id == req.plan_id) {
        p.status = plan.status;
    }

    Json(serde_json::json!({
        "plan_id": result.plan_id,
        "succeeded": result.succeeded,
        "steps_completed": result.steps_completed,
        "steps_failed": result.steps_failed,
        "error": result.error,
    }))
}

/// Destroy named resources — generates a delete plan and immediately applies it.
async fn destroy_resources(
    State(_state): State<Arc<InfraModuleState>>,
    Json(req): Json<DestroyRequest>,
) -> Json<serde_json::Value> {
    let description = format!(
        "destroy resources: {}",
        req.resource_names.join(", ")
    );
    Json(serde_json::json!({
        "message": "destroy plan queued",
        "description": description,
        "resource_count": req.resource_names.len(),
        "note": "submit to /apply with the returned plan_id to execute",
    }))
}

/// Get the current infrastructure state.
async fn get_state(State(state): State<Arc<InfraModuleState>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let s = &store.state;
    Json(serde_json::json!({
        "version": s.version,
        "resource_count": s.resources.len(),
        "last_applied": s.last_applied,
        "locked_by": s.locked_by,
        "resources": s.resources.values()
            .map(|r| serde_json::json!({
                "id": r.id,
                "name": r.name,
                "provider": r.provider,
                "type": r.resource_type,
                "state": format!("{:?}", r.state),
            }))
            .collect::<Vec<_>>(),
    }))
}

/// Detect drift between desired and actual cloud state.
async fn detect_drift(State(state): State<Arc<InfraModuleState>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let report = store.detect_drift().await;
    Json(serde_json::json!({
        "id": report.id,
        "detected_at": report.detected_at,
        "total_drifted": report.total_drifted,
        "drifted_resources": report.drifted_resources,
    }))
}

/// List all registered MCP providers.
async fn list_providers(State(state): State<Arc<InfraModuleState>>) -> Json<Vec<ProviderSummary>> {
    let registry = state.registry.lock().await;
    Json(registry.providers.iter().map(ProviderSummary::from).collect())
}

/// Register a new MCP provider (cloud integration server).
async fn register_provider(
    State(state): State<Arc<InfraModuleState>>,
    Json(req): Json<RegisterProviderRequest>,
) -> Json<serde_json::Value> {
    let mut registry = state.registry.lock().await;
    let provider = registry.register(req.name, req.endpoint);
    Json(serde_json::json!(ProviderSummary::from(&provider)))
}

/// State change history (lightweight version list).
async fn state_history(State(state): State<Arc<InfraModuleState>>) -> Json<Vec<StateSnapshot>> {
    let store = state.store.lock().await;
    Json(store.state_history().to_vec())
}

/// Import an existing cloud resource into state.
async fn import_resource(
    State(state): State<Arc<InfraModuleState>>,
    Json(req): Json<ImportRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().await;
    let resource = store.import_resource(
        req.name,
        req.provider,
        req.resource_type,
        req.actual_id,
        Default::default(),
    );
    Json(serde_json::json!({
        "id": resource.id,
        "name": resource.name,
        "provider": resource.provider,
        "type": resource.resource_type,
        "actual_id": resource.actual_id,
        "state": format!("{:?}", resource.state),
    }))
}

/// Estimate cost for an intent without generating a full plan.
async fn estimate_cost(
    State(state): State<Arc<InfraModuleState>>,
    Json(req): Json<CostRequest>,
) -> Json<serde_json::Value> {
    let infra_intent = match intent::parse_intent(&req.description, req.yaml.as_deref()) {
        Ok(i) => i,
        Err(e) => return Json(serde_json::json!({"error": e.to_string()})),
    };

    let registry = state.registry.lock().await;
    let store = state.store.lock().await;
    let plan = planner::generate_plan(&infra_intent, &store.state, &registry.providers);
    let cost = planner::estimate_cost(&plan);

    Json(serde_json::json!({
        "monthly_usd": cost.monthly_usd,
        "hourly_usd": cost.hourly_usd,
        "currency": cost.currency,
        "breakdown": cost.breakdown,
    }))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-infra",
        "status": "ok",
        "upstream": "Terraform + Crossplane",
        "approach": "LLM+MCP-native IaC",
    }))
}
