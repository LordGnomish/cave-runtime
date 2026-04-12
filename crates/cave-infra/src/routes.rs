//! HTTP routes for cave-infra.

<<<<<<< HEAD
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
=======
use crate::executor::{dry_run, execute_plan};
use serde::Serialize;
use crate::intent::{parse_intent, validate_intent};
use crate::mcp_bridge::{discover_capabilities, health_check};
use crate::models::{InfraResource, McpProvider};
use crate::planner::{estimate_cost, evaluate_policies, generate_plan};
use crate::InfraState as AppState;
use axum::{
    extract::{Path, State as AxumState},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Health
        .route("/api/infra/health", get(health))
        // Intent
        .route("/api/infra/intent", post(submit_intent))
        // Plan
        .route("/api/infra/plan", post(create_plan))
        // Apply / destroy
        .route("/api/infra/apply", post(apply_plan))
        .route("/api/infra/destroy", post(destroy_resources))
        // Dry-run
        .route("/api/infra/dry-run", post(run_dry_run))
        // State
        .route("/api/infra/state", get(get_state))
        .route("/api/infra/state/history", get(get_history))
        .route("/api/infra/state/rollback/:version", post(rollback_state))
        // Drift
        .route("/api/infra/drift", get(get_drift))
        // Providers (MCP)
        .route("/api/infra/providers", get(list_providers))
        .route("/api/infra/providers", post(register_provider))
        .route("/api/infra/providers/:id/health", get(provider_health))
        // Import
        .route("/api/infra/import", post(import_resource))
        // Cost
        .route("/api/infra/cost", post(estimate_cost_route))
        .with_state(state)
}

// ── Health ────────────────────────────────────────────────────────────────────
>>>>>>> claude/silly-matsumoto

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-infra",
        "status": "ok",
<<<<<<< HEAD
        "upstream": "Terraform + Crossplane",
        "approach": "LLM+MCP-native IaC",
=======
        "replaces": ["Terraform", "Crossplane"],
        "approach": "LLM+MCP intent-driven IaC"
    }))
}

// ── Intent ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SubmitIntentRequest {
    name: String,
    environment: String,
    /// Raw text — natural language or YAML.
    content: String,
    provider_hint: Option<String>,
}

#[derive(Debug, Serialize)]
struct SubmitIntentResponse {
    intent_id: String,
    name: String,
    environment: String,
    warnings: Vec<String>,
    is_structured: bool,
}

async fn submit_intent(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<SubmitIntentRequest>,
) -> Json<serde_json::Value> {
    let mut intent = match parse_intent(&req.content, &req.name, &req.environment) {
        Ok(i) => i,
        Err(e) => {
            return Json(serde_json::json!({ "error": e.to_string() }));
        }
    };
    intent.provider_hint = req.provider_hint;

    let warnings = validate_intent(&intent).unwrap_or_default();
    let is_structured = intent.structured.is_some();
    let intent_id = intent.id.to_string();

    state.intents.lock().await.push(intent);

    Json(serde_json::json!({
        "intent_id": intent_id,
        "name": req.name,
        "environment": req.environment,
        "warnings": warnings,
        "is_structured": is_structured,
    }))
}

// ── Plan ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreatePlanRequest {
    intent_id: String,
}

async fn create_plan(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<CreatePlanRequest>,
) -> Json<serde_json::Value> {
    let intent_id = match Uuid::parse_str(&req.intent_id) {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({ "error": "invalid intent_id" })),
    };

    let intents = state.intents.lock().await;
    let intent = match intents.iter().find(|i| i.id == intent_id) {
        Some(i) => i.clone(),
        None => return Json(serde_json::json!({ "error": "intent not found" })),
    };
    drop(intents);

    let store = state.store.lock().await;
    let infra_state = store.current.clone();
    drop(store);

    let plan = match generate_plan(&intent, &infra_state) {
        Ok(p) => p,
        Err(e) => return Json(serde_json::json!({ "error": e.to_string() })),
    };

    let policies = evaluate_policies(&plan);
    let plan_id = plan.id.to_string();
    let risk = plan.risk_score;
    let cost = plan.cost_estimate.clone();
    let explanation = plan.explanation.clone();
    let steps_count = plan.steps.len();

    state.plans.lock().await.push(plan);

    Json(serde_json::json!({
        "plan_id": plan_id,
        "steps": steps_count,
        "risk_score": risk,
        "cost_estimate": cost,
        "explanation": explanation,
        "policy_checks": policies,
    }))
}

// ── Apply ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ApplyRequest {
    plan_id: String,
}

async fn apply_plan(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<ApplyRequest>,
) -> Json<serde_json::Value> {
    let plan_id = match Uuid::parse_str(&req.plan_id) {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({ "error": "invalid plan_id" })),
    };

    let plans = state.plans.lock().await;
    let plan = match plans.iter().find(|p| p.id == plan_id) {
        Some(p) => p.clone(),
        None => return Json(serde_json::json!({ "error": "plan not found" })),
    };
    drop(plans);

    let exec = match execute_plan(&plan, Arc::clone(&state.registry), Arc::clone(&state.store)).await {
        Ok(e) => e,
        Err(e) => return Json(serde_json::json!({ "error": e.to_string() })),
    };

    Json(serde_json::json!({
        "execution_id": exec.id.to_string(),
        "plan_id": plan_id.to_string(),
        "succeeded": exec.succeeded,
        "steps_total": exec.steps.len(),
        "steps_succeeded": exec.steps.iter().filter(|s| matches!(s.status, crate::executor::StepStatus::Succeeded)).count(),
        "steps_failed": exec.steps.iter().filter(|s| matches!(s.status, crate::executor::StepStatus::Failed)).count(),
        "dry_run": false,
    }))
}

// ── Destroy ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct DestroyRequest {
    resource_names: Vec<String>,
}

async fn destroy_resources(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<DestroyRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().await;
    for name in &req.resource_names {
        let _ = store.remove_desired(name, format!("destroy '{}'", name));
    }
    Json(serde_json::json!({
        "destroyed": req.resource_names,
        "status": "queued — run apply to reconcile",
    }))
}

// ── Dry Run ───────────────────────────────────────────────────────────────────

async fn run_dry_run(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<ApplyRequest>,
) -> Json<serde_json::Value> {
    let plan_id = match Uuid::parse_str(&req.plan_id) {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({ "error": "invalid plan_id" })),
    };

    let plans = state.plans.lock().await;
    let plan = match plans.iter().find(|p| p.id == plan_id) {
        Some(p) => p.clone(),
        None => return Json(serde_json::json!({ "error": "plan not found" })),
    };
    drop(plans);

    let exec = match dry_run(&plan, Arc::clone(&state.registry), Arc::clone(&state.store)).await {
        Ok(e) => e,
        Err(e) => return Json(serde_json::json!({ "error": e.to_string() })),
    };

    Json(serde_json::json!({
        "execution_id": exec.id.to_string(),
        "plan_id": plan_id.to_string(),
        "dry_run": true,
        "would_succeed": exec.succeeded,
        "steps": exec.steps.len(),
    }))
}

// ── State ─────────────────────────────────────────────────────────────────────

async fn get_state(AxumState(state): AxumState<Arc<AppState>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    Json(serde_json::json!({
        "version": store.current.version,
        "locked": store.current.locked,
        "lock_holder": store.current.lock_holder,
        "desired_count": store.current.desired.len(),
        "actual_count": store.current.actual.len(),
        "last_synced": store.current.last_synced,
    }))
}

async fn get_history(AxumState(state): AxumState<Arc<AppState>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let history: Vec<serde_json::Value> = store
        .state_history()
        .iter()
        .map(|s| {
            serde_json::json!({
                "version": s.version,
                "comment": s.comment,
                "taken_at": s.taken_at,
            })
        })
        .collect();
    Json(serde_json::json!({ "history": history }))
}

async fn rollback_state(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(version): Path<u64>,
) -> Json<serde_json::Value> {
    let mut store = state.store.lock().await;
    match store.rollback_to_version(version) {
        Ok(()) => Json(serde_json::json!({ "status": "ok", "rolled_back_to": version })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ── Drift ─────────────────────────────────────────────────────────────────────

async fn get_drift(AxumState(state): AxumState<Arc<AppState>>) -> Json<serde_json::Value> {
    let store = state.store.lock().await;
    let report = store.detect_drift();
    Json(serde_json::json!({
        "report_id": report.id.to_string(),
        "drifted": report.drifted.len(),
        "missing": report.missing,
        "orphaned": report.orphaned,
        "items": report.drifted,
        "generated_at": report.generated_at,
    }))
}

// ── Providers ─────────────────────────────────────────────────────────────────

async fn list_providers(AxumState(state): AxumState<Arc<AppState>>) -> Json<serde_json::Value> {
    let reg = state.registry.lock().await;
    let providers: Vec<serde_json::Value> = reg
        .list()
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id.to_string(),
                "name": p.name,
                "provider": p.provider,
                "endpoint": p.endpoint,
                "healthy": p.healthy,
                "tools_count": p.tools.len(),
                "capabilities": p.capabilities,
            })
        })
        .collect();
    Json(serde_json::json!({ "providers": providers }))
}

#[derive(Debug, Deserialize)]
struct RegisterProviderRequest {
    name: String,
    provider: String,
    endpoint: String,
}

async fn register_provider(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<RegisterProviderRequest>,
) -> Json<serde_json::Value> {
    let mut provider = McpProvider::new(&req.name, &req.provider, &req.endpoint);

    // Discover capabilities from the endpoint.
    match discover_capabilities(&req.endpoint).await {
        Ok((tools, caps)) => {
            provider.tools = tools;
            provider.capabilities = caps;
            provider.healthy = true;
        }
        Err(_) => {
            provider.healthy = false;
        }
    }

    let provider_id = provider.id.to_string();
    let healthy = provider.healthy;
    state.registry.lock().await.register(provider);

    Json(serde_json::json!({
        "provider_id": provider_id,
        "name": req.name,
        "provider": req.provider,
        "healthy": healthy,
        "status": "registered",
    }))
}

async fn provider_health(
    AxumState(state): AxumState<Arc<AppState>>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    let provider_id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({ "error": "invalid provider id" })),
    };

    let endpoint = {
        let reg = state.registry.lock().await;
        reg.list()
            .iter()
            .find(|p| p.id == provider_id)
            .map(|p| p.endpoint.clone())
    };

    match endpoint {
        None => Json(serde_json::json!({ "error": "provider not found" })),
        Some(ep) => {
            let healthy = health_check(&ep).await;
            state.registry.lock().await.set_health(provider_id, healthy);
            Json(serde_json::json!({ "provider_id": id, "healthy": healthy }))
        }
    }
}

// ── Import ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ImportRequest {
    name: String,
    provider: String,
    resource_type: String,
    remote_id: String,
    config: Option<serde_json::Value>,
}

async fn import_resource(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<ImportRequest>,
) -> Json<serde_json::Value> {
    let mut resource = InfraResource::new(&req.name, &req.provider, &req.resource_type);
    if let Some(cfg) = req.config {
        if let Some(obj) = cfg.as_object() {
            for (k, v) in obj {
                resource.config.insert(k.clone(), v.clone());
            }
        }
    }

    let resource_id = resource.id.to_string();
    let mut store = state.store.lock().await;
    match store.import_resource(resource, &req.remote_id) {
        Ok(()) => Json(serde_json::json!({
            "resource_id": resource_id,
            "name": req.name,
            "remote_id": req.remote_id,
            "status": "imported",
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

// ── Cost ──────────────────────────────────────────────────────────────────────

async fn estimate_cost_route(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<ApplyRequest>,
) -> Json<serde_json::Value> {
    let plan_id = match Uuid::parse_str(&req.plan_id) {
        Ok(id) => id,
        Err(_) => return Json(serde_json::json!({ "error": "invalid plan_id" })),
    };

    let plans = state.plans.lock().await;
    let plan = match plans.iter().find(|p| p.id == plan_id) {
        Some(p) => p.clone(),
        None => return Json(serde_json::json!({ "error": "plan not found" })),
    };
    drop(plans);

    let cost = estimate_cost(&plan);
    Json(serde_json::json!({
        "plan_id": plan_id.to_string(),
        "monthly_usd": cost.monthly_usd,
        "breakdown": cost.breakdown,
        "confidence": cost.confidence,
        "currency": cost.currency,
        "notes": cost.notes,
>>>>>>> claude/silly-matsumoto
    }))
}
