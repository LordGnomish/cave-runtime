// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::Utc;
use uuid::Uuid;

use crate::{
    CostAllocState, allocator,
    models::{
        BudgetPolicy, ChargebackQuery, ChargebackRule, CostAnomaly, CostCenter,
        CreateBudgetPolicyRequest, CreateChargebackRuleRequest, CreateCostCenterRequest,
        ForecastModel, ForecastQuery, IdleResource, Invoice, ShowbackQuery, ShowbackReport,
        UnitEconomics,
    },
    reporting,
};

type ApiError = (StatusCode, Json<serde_json::Value>);
type ApiResult<T> = Result<Json<T>, ApiError>;

fn not_found(msg: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg })),
    )
}

pub fn create_router(state: Arc<CostAllocState>) -> Router {
    Router::new()
        // Cost Centers
        .route(
            "/api/v1/finops/cost-centers",
            get(list_cost_centers).post(create_cost_center),
        )
        .route(
            "/api/v1/finops/cost-centers/{id}",
            get(get_cost_center)
                .put(update_cost_center)
                .delete(delete_cost_center),
        )
        // Budgets
        .route(
            "/api/v1/finops/budgets",
            get(list_budgets).post(create_budget),
        )
        .route(
            "/api/v1/finops/budgets/{id}",
            get(get_budget).delete(delete_budget),
        )
        // Allocation / chargeback rules
        .route(
            "/api/v1/finops/allocation-rules",
            get(list_allocation_rules).post(create_allocation_rule),
        )
        .route(
            "/api/v1/finops/allocation-rules/{id}",
            get(get_allocation_rule).delete(delete_allocation_rule),
        )
        // Reports & analytics
        .route("/api/v1/finops/showback", get(showback))
        .route("/api/v1/finops/chargeback", get(chargeback))
        .route("/api/v1/finops/forecast", get(forecast))
        .route("/api/v1/finops/anomalies", get(anomalies))
        .route("/api/v1/finops/idle-resources", get(idle_resources))
        .route("/api/v1/finops/unit-economics", get(unit_economics))
        // Health
        .route("/api/v1/finops/health", get(health))
        .with_state(state)
}

// --- Cost Centers ---

async fn list_cost_centers(State(state): State<Arc<CostAllocState>>) -> Json<Vec<CostCenter>> {
    Json(state.store.lock().unwrap().cost_centers.clone())
}

async fn create_cost_center(
    State(state): State<Arc<CostAllocState>>,
    Json(req): Json<CreateCostCenterRequest>,
) -> Json<CostCenter> {
    let now = Utc::now();
    let cc = CostCenter {
        id: Uuid::new_v4(),
        name: req.name,
        team: req.team,
        project: req.project,
        department: req.department,
        budget_usd: req.budget_usd,
        owner_email: req.owner_email,
        tags: req.tags,
        created_at: now,
        updated_at: now,
    };
    state.store.lock().unwrap().cost_centers.push(cc.clone());
    Json(cc)
}

async fn get_cost_center(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<CostCenter> {
    state
        .store
        .lock()
        .unwrap()
        .cost_centers
        .iter()
        .find(|cc| cc.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("cost center not found"))
}

async fn update_cost_center(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
    Json(req): Json<CreateCostCenterRequest>,
) -> ApiResult<CostCenter> {
    let mut store = state.store.lock().unwrap();
    let cc = store
        .cost_centers
        .iter_mut()
        .find(|cc| cc.id == id)
        .ok_or_else(|| not_found("cost center not found"))?;
    cc.name = req.name;
    cc.team = req.team;
    cc.project = req.project;
    cc.department = req.department;
    cc.budget_usd = req.budget_usd;
    cc.owner_email = req.owner_email;
    cc.tags = req.tags;
    cc.updated_at = Utc::now();
    Ok(Json(cc.clone()))
}

async fn delete_cost_center(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    let before = store.cost_centers.len();
    store.cost_centers.retain(|cc| cc.id != id);
    if store.cost_centers.len() < before {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// --- Budgets ---

async fn list_budgets(State(state): State<Arc<CostAllocState>>) -> Json<Vec<BudgetPolicy>> {
    Json(state.store.lock().unwrap().budget_policies.clone())
}

async fn create_budget(
    State(state): State<Arc<CostAllocState>>,
    Json(req): Json<CreateBudgetPolicyRequest>,
) -> Json<BudgetPolicy> {
    let now = Utc::now();
    let policy = BudgetPolicy {
        id: Uuid::new_v4(),
        cost_center_id: req.cost_center_id,
        period: req.period,
        limit_usd: req.limit_usd,
        alert_threshold_pct: req.alert_threshold_pct,
        hard_cap: req.hard_cap,
        auto_scale_cap_pct: req.auto_scale_cap_pct,
        created_at: now,
        updated_at: now,
    };
    state
        .store
        .lock()
        .unwrap()
        .budget_policies
        .push(policy.clone());
    Json(policy)
}

async fn get_budget(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<BudgetPolicy> {
    state
        .store
        .lock()
        .unwrap()
        .budget_policies
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("budget policy not found"))
}

async fn delete_budget(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    let before = store.budget_policies.len();
    store.budget_policies.retain(|p| p.id != id);
    if store.budget_policies.len() < before {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// --- Allocation Rules ---

async fn list_allocation_rules(
    State(state): State<Arc<CostAllocState>>,
) -> Json<Vec<ChargebackRule>> {
    Json(state.store.lock().unwrap().chargeback_rules.clone())
}

async fn create_allocation_rule(
    State(state): State<Arc<CostAllocState>>,
    Json(req): Json<CreateChargebackRuleRequest>,
) -> Json<ChargebackRule> {
    let rule = ChargebackRule {
        id: Uuid::new_v4(),
        name: req.name,
        resource_type: req.resource_type,
        split_strategy: req.split_strategy,
        cost_center_ids: req.cost_center_ids,
        created_at: Utc::now(),
    };
    state
        .store
        .lock()
        .unwrap()
        .chargeback_rules
        .push(rule.clone());
    Json(rule)
}

async fn get_allocation_rule(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
) -> ApiResult<ChargebackRule> {
    state
        .store
        .lock()
        .unwrap()
        .chargeback_rules
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .map(Json)
        .ok_or_else(|| not_found("allocation rule not found"))
}

async fn delete_allocation_rule(
    State(state): State<Arc<CostAllocState>>,
    Path(id): Path<Uuid>,
) -> StatusCode {
    let mut store = state.store.lock().unwrap();
    let before = store.chargeback_rules.len();
    store.chargeback_rules.retain(|r| r.id != id);
    if store.chargeback_rules.len() < before {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

// --- Reports & Analytics ---

async fn showback(
    State(state): State<Arc<CostAllocState>>,
    Query(q): Query<ShowbackQuery>,
) -> Json<Vec<ShowbackReport>> {
    let store = state.store.lock().unwrap();
    let reports = reporting::generate_showback(&store.cost_centers, &[]);
    let filtered = reports
        .into_iter()
        .filter(|r| q.team.as_ref().map_or(true, |t| &r.team == t))
        .collect();
    Json(filtered)
}

async fn chargeback(
    State(state): State<Arc<CostAllocState>>,
    Query(q): Query<ChargebackQuery>,
) -> Json<Vec<Invoice>> {
    let store = state.store.lock().unwrap();

    let cc_ids: HashSet<Uuid> = if let Some(team) = &q.team {
        store
            .cost_centers
            .iter()
            .filter(|cc| &cc.team == team)
            .map(|cc| cc.id)
            .collect()
    } else {
        store.cost_centers.iter().map(|cc| cc.id).collect()
    };

    let cost_centers: Vec<CostCenter> = store
        .cost_centers
        .iter()
        .filter(|cc| cc_ids.contains(&cc.id))
        .cloned()
        .collect();

    drop(store);
    Json(reporting::generate_chargeback(&cost_centers, &[]))
}

async fn forecast(
    State(state): State<Arc<CostAllocState>>,
    Query(q): Query<ForecastQuery>,
) -> Json<Vec<ForecastModel>> {
    let months = q.months.unwrap_or(3).clamp(1, 24);
    let store = state.store.lock().unwrap();
    let forecasts = store
        .cost_centers
        .iter()
        .map(|cc| reporting::forecast_spending(cc.id, &[], months))
        .collect();
    Json(forecasts)
}

async fn anomalies(State(_state): State<Arc<CostAllocState>>) -> Json<Vec<CostAnomaly>> {
    // Anomaly detection operates over historical CostReports fetched from the DB
    // (cave-db integration is future work); returns an empty set until wired up.
    Json(allocator::detect_anomalies(&[], 30.0))
}

async fn idle_resources(State(_state): State<Arc<CostAllocState>>) -> Json<Vec<IdleResource>> {
    // Cluster utilization data will be provided by cave-metrics integration.
    Json(allocator::calculate_idle_costs(&[], 10.0))
}

async fn unit_economics(State(_state): State<Arc<CostAllocState>>) -> Json<UnitEconomics> {
    Json(reporting::unit_economics(&[], 0, 0, 0))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "module": "cave-cost-alloc",
        "status": "ok",
        "upstream": ["Kubecost", "CloudHealth"],
    }))
}
