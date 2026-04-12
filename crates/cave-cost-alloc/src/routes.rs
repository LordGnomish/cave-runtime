//! HTTP routes for cave-cost-alloc.

use crate::models::*;
use crate::store::CostAllocStore;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::sync::Arc;

pub fn create_router(store: Arc<CostAllocStore>) -> Router {
    Router::new()
        .route("/api/cost/health", get(health))
        .route("/api/cost/allocations", get(list_allocations))
        .route("/api/cost/allocations", post(add_allocation))
        .route("/api/cost/showback", get(showback))
        .route("/api/cost/chargeback", get(chargeback))
        .route("/api/cost/budgets", get(list_budgets))
        .route("/api/cost/budgets", post(create_budget))
        .route("/api/cost/efficiency", get(efficiency))
        .route("/api/cost/recommendations", get(get_recommendations))
        .route("/api/cost/recommendations/refresh", post(refresh_recommendations))
        .route("/api/cost/cloud", get(list_cloud))
        .route("/api/cost/cloud", post(add_cloud_cost))
        .route("/api/cost/summary", get(summary))
        .with_state(store)
}

// ─── Query params ─────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct AllocQuery {
    namespace: Option<String>,
    team: Option<String>,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Deserialize, Default)]
struct ShowbackQuery {
    group_by: Option<String>,
}

#[derive(Deserialize, Default)]
struct EfficiencyQuery {
    namespace: Option<String>,
}

#[derive(Deserialize, Default)]
struct CloudQuery {
    provider: Option<String>,
}

fn parse_provider(s: &str) -> Option<CloudProvider> {
    match s.to_lowercase().as_str() {
        "aws" => Some(CloudProvider::Aws),
        "azure" => Some(CloudProvider::Azure),
        "gcp" => Some(CloudProvider::Gcp),
        _ => None,
    }
}

// ─── Handlers ────────────────────────────────────────────────────────────────

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cost-alloc",
        "status": "ok",
        "upstream": "Kubecost, OpenCost"
    }))
}

async fn list_allocations(
    State(store): State<Arc<CostAllocStore>>,
    Query(q): Query<AllocQuery>,
) -> Json<Vec<CostAllocation>> {
    let query = AllocationQuery {
        namespace: q.namespace,
        team: q.team,
        start: q.start,
        end: q.end,
    };
    Json(store.list_allocations(&query))
}

async fn add_allocation(
    State(store): State<Arc<CostAllocStore>>,
    Json(req): Json<CreateAllocationRequest>,
) -> (StatusCode, Json<CostAllocation>) {
    let alloc = store.add_allocation(req);
    (StatusCode::CREATED, Json(alloc))
}

async fn showback(
    State(store): State<Arc<CostAllocStore>>,
    Query(q): Query<ShowbackQuery>,
) -> Json<ShowbackReport> {
    let group_by = q.group_by.as_deref().unwrap_or("namespace");
    Json(store.generate_showback_report(group_by))
}

async fn chargeback(State(store): State<Arc<CostAllocStore>>) -> Json<ChargebackReport> {
    Json(store.generate_chargeback_report())
}

async fn list_budgets(State(store): State<Arc<CostAllocStore>>) -> Json<Vec<BudgetAlert>> {
    Json(store.list_budgets())
}

async fn create_budget(
    State(store): State<Arc<CostAllocStore>>,
    Json(req): Json<CreateBudgetAlertRequest>,
) -> (StatusCode, Json<BudgetAlert>) {
    let alert = store.create_budget(req);
    (StatusCode::CREATED, Json(alert))
}

async fn efficiency(
    State(store): State<Arc<CostAllocStore>>,
    Query(q): Query<EfficiencyQuery>,
) -> Json<Vec<EfficiencyReport>> {
    Json(store.efficiency_report(q.namespace.as_deref()))
}

async fn get_recommendations(
    State(store): State<Arc<CostAllocStore>>,
) -> Json<Vec<Recommendation>> {
    Json(store.get_recommendations())
}

async fn refresh_recommendations(
    State(store): State<Arc<CostAllocStore>>,
) -> Json<Vec<Recommendation>> {
    Json(store.refresh_recommendations())
}

async fn list_cloud(
    State(store): State<Arc<CostAllocStore>>,
    Query(q): Query<CloudQuery>,
) -> Json<Vec<CloudCostEntry>> {
    let provider = q.provider.as_deref().and_then(parse_provider);
    Json(store.list_cloud_costs(provider))
}

async fn add_cloud_cost(
    State(store): State<Arc<CostAllocStore>>,
    Json(req): Json<AddCloudCostRequest>,
) -> (StatusCode, Json<CloudCostEntry>) {
    let entry = store.add_cloud_cost(req);
    (StatusCode::CREATED, Json(entry))
}

async fn summary(State(store): State<Arc<CostAllocStore>>) -> Json<serde_json::Value> {
    Json(store.summary())
}
