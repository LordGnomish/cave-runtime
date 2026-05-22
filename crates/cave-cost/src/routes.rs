// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTTP routes for cave-cost (FinOps / cost tracking).

use crate::models::*;
use crate::{CostState, CostStore};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

// ── Request / response DTOs ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CostQueryRequest {
    pub namespace: Option<String>,
    pub window: Option<ReportWindow>,
    pub aggregate_by: Option<AggregateBy>,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreatePricingRequest {
    pub name: String,
    pub provider: CloudProvider,
    pub cpu_core_hour: Option<f64>,
    pub memory_gb_hour: Option<f64>,
    pub storage_gb_month: Option<f64>,
    pub network_egress_gb: Option<f64>,
    pub gpu_core_hour: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportRequest {
    pub name: String,
    pub window: ReportWindow,
    pub aggregate_by: AggregateBy,
    pub window_start: Option<DateTime<Utc>>,
    pub window_end: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBudgetRequest {
    pub name: String,
    pub namespace: Option<String>,
    pub label_selector: Option<HashMap<String, String>>,
    pub monthly_limit_usd: f64,
    pub alert_threshold_percent: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct AllocationQuery {
    pub namespace: Option<String>,
    pub window: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ShowbackQuery {
    pub report_type: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct TrendQuery {
    pub namespace: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn create_router(state: Arc<CostState>) -> Router {
    Router::new()
        // Cost queries & allocations
        .route("/api/cost/query", post(query_costs))
        .route("/api/cost/allocations", get(get_allocations))
        // Pricing
        .route("/api/cost/pricing", post(create_pricing))
        .route("/api/cost/pricing", get(list_pricing))
        .route("/api/cost/pricing/{id}", get(get_pricing))
        .route("/api/cost/pricing/{id}", delete(delete_pricing))
        // Reports
        .route("/api/cost/reports", post(create_report))
        .route("/api/cost/reports", get(list_reports))
        .route("/api/cost/reports/{id}", get(get_report))
        .route("/api/cost/reports/{id}/export", get(export_report))
        // Showback / chargeback
        .route("/api/cost/showback", get(get_showback))
        // Budgets
        .route("/api/cost/budgets", post(create_budget))
        .route("/api/cost/budgets", get(list_budgets))
        .route("/api/cost/budgets/{id}", get(get_budget))
        .route("/api/cost/budgets/{id}", delete(delete_budget))
        .route(
            "/api/cost/budgets/{id}/evaluate",
            post(evaluate_budget_handler),
        )
        // Alerts
        .route("/api/cost/alerts", get(list_alerts))
        // Recommendations
        .route("/api/cost/recommendations", get(list_recommendations))
        .route(
            "/api/cost/recommendations/refresh",
            post(refresh_recommendations),
        )
        // Trends
        .route("/api/cost/trends", get(get_trends))
        // Health
        .route("/api/cost/health", get(health))
        .with_state(state)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn _active_pricing(store: &CostStore) -> Option<PricingConfig> {
    store
        .pricing_configs
        .values()
        .find(|p| p.name == "default")
        .or_else(|| store.pricing_configs.values().next())
        .cloned()
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// POST /api/cost/query
async fn query_costs(
    State(state): State<Arc<CostState>>,
    Json(req): Json<CostQueryRequest>,
) -> Json<Vec<CostAllocation>> {
    let store = state.store.read().await;
    let window = req.window.unwrap_or(ReportWindow::LastDay);
    let (window_start, window_end) =
        crate::reports::window_bounds(&window, req.window_start, req.window_end);
    let aggregate_by = req.aggregate_by.unwrap_or(AggregateBy::Namespace);

    let filtered: Vec<ResourceCost> = store
        .resource_costs
        .iter()
        .filter(|c| req.namespace.as_ref().map_or(true, |ns| &c.namespace == ns))
        .cloned()
        .collect();

    let allocations =
        crate::allocation::aggregate_costs(&filtered, &aggregate_by, window_start, window_end);
    Json(allocations)
}

/// GET /api/cost/allocations
async fn get_allocations(
    State(state): State<Arc<CostState>>,
    Query(params): Query<AllocationQuery>,
) -> Json<Vec<CostAllocation>> {
    let store = state.store.read().await;
    let window_str = params.window.as_deref().unwrap_or("last_day");
    let report_window = match window_str {
        "last_week" => ReportWindow::LastWeek,
        "last_month" => ReportWindow::LastMonth,
        _ => ReportWindow::LastDay,
    };
    let (window_start, window_end) = crate::reports::window_bounds(&report_window, None, None);

    let filtered: Vec<ResourceCost> = store
        .resource_costs
        .iter()
        .filter(|c| {
            params
                .namespace
                .as_ref()
                .map_or(true, |ns| &c.namespace == ns)
        })
        .cloned()
        .collect();

    let allocations = crate::allocation::aggregate_costs(
        &filtered,
        &AggregateBy::Namespace,
        window_start,
        window_end,
    );
    Json(allocations)
}

// ── Pricing ───────────────────────────────────────────────────────────────────

/// POST /api/cost/pricing
async fn create_pricing(
    State(state): State<Arc<CostState>>,
    Json(req): Json<CreatePricingRequest>,
) -> Json<PricingConfig> {
    let mut config = crate::pricing::default_config_for_provider(&req.name, req.provider);
    if let Some(v) = req.cpu_core_hour {
        config.cpu_core_hour = v;
    }
    if let Some(v) = req.memory_gb_hour {
        config.memory_gb_hour = v;
    }
    if let Some(v) = req.storage_gb_month {
        config.storage_gb_month = v;
    }
    if let Some(v) = req.network_egress_gb {
        config.network_egress_gb = v;
    }
    if let Some(v) = req.gpu_core_hour {
        config.gpu_core_hour = v;
    }
    let mut store = state.store.write().await;
    store.pricing_configs.insert(config.id, config.clone());
    Json(config)
}

/// GET /api/cost/pricing
async fn list_pricing(State(state): State<Arc<CostState>>) -> Json<Vec<PricingConfig>> {
    let store = state.store.read().await;
    Json(store.pricing_configs.values().cloned().collect())
}

/// GET /api/cost/pricing/:id
async fn get_pricing(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    match store.pricing_configs.get(&id) {
        Some(config) => Json(serde_json::json!(config)),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

/// DELETE /api/cost/pricing/:id
async fn delete_pricing(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.write().await;
    if store.pricing_configs.remove(&id).is_some() {
        Json(serde_json::json!({ "deleted": id }))
    } else {
        Json(serde_json::json!({ "error": "not found" }))
    }
}

// ── Reports ───────────────────────────────────────────────────────────────────

/// POST /api/cost/reports
async fn create_report(
    State(state): State<Arc<CostState>>,
    Json(req): Json<CreateReportRequest>,
) -> Json<CostReport> {
    let (window_start, window_end) =
        crate::reports::window_bounds(&req.window, req.window_start, req.window_end);

    let (filtered, aggregate_by) = {
        let store_r = state.store.read().await;
        let filtered: Vec<ResourceCost> = store_r
            .resource_costs
            .iter()
            .filter(|c| c.window_start >= window_start && c.window_end <= window_end)
            .cloned()
            .collect();
        (filtered, req.aggregate_by)
    };

    let allocations =
        crate::allocation::aggregate_costs(&filtered, &aggregate_by, window_start, window_end);

    let report = crate::reports::build_report(
        req.name,
        req.window,
        window_start,
        window_end,
        aggregate_by,
        allocations,
    );

    let mut store = state.store.write().await;
    store.cost_reports.insert(report.id, report.clone());
    Json(report)
}

/// GET /api/cost/reports
async fn list_reports(State(state): State<Arc<CostState>>) -> Json<Vec<CostReport>> {
    let store = state.store.read().await;
    Json(store.cost_reports.values().cloned().collect())
}

/// GET /api/cost/reports/:id
async fn get_report(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    match store.cost_reports.get(&id) {
        Some(r) => Json(serde_json::json!(r)),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

/// GET /api/cost/reports/:id/export — return the report as JSON
async fn export_report(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    match store.cost_reports.get(&id) {
        Some(r) => Json(serde_json::json!(r)),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

// ── Showback ──────────────────────────────────────────────────────────────────

/// GET /api/cost/showback
async fn get_showback(
    State(state): State<Arc<CostState>>,
    Query(params): Query<ShowbackQuery>,
) -> Json<ShowbackReport> {
    let report_type = match params.report_type.as_deref() {
        Some("chargeback") => ShowbackType::Chargeback,
        _ => ShowbackType::Showback,
    };
    let store = state.store.read().await;
    let (window_start, window_end) =
        crate::reports::window_bounds(&ReportWindow::LastMonth, None, None);
    let allocations = crate::allocation::aggregate_costs(
        &store.resource_costs,
        &AggregateBy::Namespace,
        window_start,
        window_end,
    );
    let report = crate::reports::build_showback_report(
        "showback".to_string(),
        report_type,
        window_start,
        window_end,
        &allocations,
    );
    Json(report)
}

// ── Budgets ───────────────────────────────────────────────────────────────────

/// POST /api/cost/budgets
async fn create_budget(
    State(state): State<Arc<CostState>>,
    Json(req): Json<CreateBudgetRequest>,
) -> Json<Budget> {
    let budget = Budget {
        id: Uuid::new_v4(),
        name: req.name,
        namespace: req.namespace,
        label_selector: req.label_selector.unwrap_or_default(),
        monthly_limit_usd: req.monthly_limit_usd,
        alert_threshold_percent: req.alert_threshold_percent.unwrap_or(80.0),
        alert_trend_percent: None,
        current_spend: 0.0,
        forecasted_spend: 0.0,
        status: BudgetStatus::Ok,
        created_at: Utc::now(),
    };
    let mut store = state.store.write().await;
    store.budgets.insert(budget.id, budget.clone());
    Json(budget)
}

/// GET /api/cost/budgets
async fn list_budgets(State(state): State<Arc<CostState>>) -> Json<Vec<Budget>> {
    let store = state.store.read().await;
    Json(store.budgets.values().cloned().collect())
}

/// GET /api/cost/budgets/:id
async fn get_budget(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    match store.budgets.get(&id) {
        Some(b) => Json(serde_json::json!(b)),
        None => Json(serde_json::json!({ "error": "not found" })),
    }
}

/// DELETE /api/cost/budgets/:id
async fn delete_budget(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let mut store = state.store.write().await;
    if store.budgets.remove(&id).is_some() {
        Json(serde_json::json!({ "deleted": id }))
    } else {
        Json(serde_json::json!({ "error": "not found" }))
    }
}

/// POST /api/cost/budgets/:id/evaluate
async fn evaluate_budget_handler(
    State(state): State<Arc<CostState>>,
    Path(id): Path<Uuid>,
) -> Json<EvaluateBudgetResponse> {
    let mut store = state.store.write().await;
    match store.budgets.get_mut(&id) {
        Some(budget) => {
            let alerts = crate::budget::evaluate_budget(budget);
            let status = budget.status.clone();
            store.budget_alerts.extend(alerts.clone());
            Json(EvaluateBudgetResponse { status, alerts })
        }
        None => Json(EvaluateBudgetResponse {
            status: BudgetStatus::Ok,
            alerts: vec![],
        }),
    }
}

#[derive(Serialize)]
pub struct EvaluateBudgetResponse {
    pub status: BudgetStatus,
    pub alerts: Vec<BudgetAlert>,
}

// ── Alerts ────────────────────────────────────────────────────────────────────

/// GET /api/cost/alerts
async fn list_alerts(State(state): State<Arc<CostState>>) -> Json<Vec<BudgetAlert>> {
    let store = state.store.read().await;
    Json(store.budget_alerts.clone())
}

// ── Recommendations ───────────────────────────────────────────────────────────

/// GET /api/cost/recommendations
async fn list_recommendations(
    State(state): State<Arc<CostState>>,
) -> Json<Vec<CostRecommendation>> {
    let store = state.store.read().await;
    Json(store.recommendations.values().cloned().collect())
}

/// POST /api/cost/recommendations/refresh
async fn refresh_recommendations(
    State(state): State<Arc<CostState>>,
) -> Json<RefreshRecommendationsResponse> {
    let costs = {
        let store_r = state.store.read().await;
        store_r.resource_costs.clone()
    };

    let rightsizing = crate::recommendations::rightsizing_recommendations(&costs);
    let orphaned = crate::recommendations::orphaned_resource_recommendations(&costs);
    let all = crate::recommendations::merge_recommendations(rightsizing, orphaned);

    let mut store = state.store.write().await;
    store.recommendations.clear();
    for rec in &all {
        store.recommendations.insert(rec.id, rec.clone());
    }

    Json(RefreshRecommendationsResponse {
        count: all.len(),
        estimated_total_savings: all.iter().map(|r| r.estimated_savings).sum(),
    })
}

#[derive(Serialize)]
pub struct RefreshRecommendationsResponse {
    pub count: usize,
    pub estimated_total_savings: f64,
}

// ── Trends ────────────────────────────────────────────────────────────────────

/// GET /api/cost/trends
async fn get_trends(
    State(state): State<Arc<CostState>>,
    Query(params): Query<TrendQuery>,
) -> Json<CostTrend> {
    let store = state.store.read().await;

    let filtered: Vec<&ResourceCost> = store
        .resource_costs
        .iter()
        .filter(|c| {
            params
                .namespace
                .as_ref()
                .map_or(true, |ns| &c.namespace == ns)
        })
        .collect();

    // Aggregate by day (keyed on window_start date string)
    let mut by_day: HashMap<String, f64> = HashMap::new();
    for cost in &filtered {
        let day_key = cost.window_start.format("%Y-%m-%d").to_string();
        *by_day.entry(day_key).or_insert(0.0) += cost.total_cost;
    }

    let mut sorted_days: Vec<(String, f64)> = by_day.into_iter().collect();
    sorted_days.sort_by(|a, b| a.0.cmp(&b.0));

    let cost_series: Vec<(DateTime<Utc>, f64)> = sorted_days
        .into_iter()
        .filter_map(|(day, cost)| {
            chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|ndt| (DateTime::from_naive_utc_and_offset(ndt, Utc), cost))
        })
        .collect();

    let trend = crate::reports::generate_trend(cost_series, params.namespace);
    Json(trend)
}

// ── Health ────────────────────────────────────────────────────────────────────

/// GET /api/cost/health
async fn health(State(state): State<Arc<CostState>>) -> Json<serde_json::Value> {
    let store = state.store.read().await;
    Json(serde_json::json!({
        "module": "cave-cost",
        "status": "ok",
        "upstream": "opencost",
        "pricing_configs": store.pricing_configs.len(),
        "budgets": store.budgets.len(),
        "recommendations": store.recommendations.len(),
        "resource_costs_tracked": store.resource_costs.len(),
    }))
}
