// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::ar::{PartnerBalance, compute_partner_balance};
use crate::payroll::{Allowance, Deduction, TaxBracket, compute_payslip};
use crate::store::ErpStore;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

async fn health() -> impl IntoResponse {
    Json(json!({
        "module": "cave-erp",
        "status": "ok",
        "upstream": "Odoo Community Edition",
        "submodules": [
            "hr",
            "recruitment",
            "sales",
            "purchase",
            "inventory",
            "accounting",
            "manufacturing",
            "projects",
            "payroll",
            "ar"
        ]
    }))
}

/// Request body for computing a payslip.
#[derive(Serialize, Deserialize)]
pub struct ComputePayslipRequest {
    pub base_salary: f64,
    pub currency: String,
    pub allowances: Vec<Allowance>,
    pub deductions: Vec<Deduction>,
    pub brackets: Vec<TaxBracket>,
}

/// POST /api/erp/payroll/employees/{id}/payslip
async fn compute_payslip_handler(
    State(_store): State<Arc<ErpStore>>,
    Path(employee_id): Path<Uuid>,
    Json(req): Json<ComputePayslipRequest>,
) -> impl IntoResponse {
    let slip = compute_payslip(
        employee_id,
        req.base_salary,
        &req.allowances,
        &req.deductions,
        &req.brackets,
        &req.currency,
    );
    (StatusCode::OK, Json(slip))
}

/// GET /api/erp/ar/partners/{id}/balance
async fn get_partner_balance(
    State(store): State<Arc<ErpStore>>,
    Path(partner_id): Path<Uuid>,
) -> impl IntoResponse {
    let invoices: Vec<_> = store
        .invoices
        .read()
        .await
        .values()
        .cloned()
        .collect();
    let payments: Vec<_> = store
        .payments
        .read()
        .await
        .values()
        .cloned()
        .collect();

    let balance: PartnerBalance = compute_partner_balance(partner_id, &invoices, &payments);
    Json(balance)
}

pub fn create_router(state: Arc<ErpStore>) -> Router {
    let core_routes = Router::new()
        .route("/api/erp/health", get(health))
        // Payroll
        .route(
            "/api/erp/payroll/employees/{id}/payslip",
            post(compute_payslip_handler),
        )
        // Accounts Receivable
        .route(
            "/api/erp/ar/partners/{id}/balance",
            get(get_partner_balance),
        )
        .with_state(state.clone());

    core_routes
        .merge(crate::modules::hr::create_router(state.clone()))
        .merge(crate::modules::recruitment::create_router(state.clone()))
        .merge(crate::modules::sales::create_router(state.clone()))
        .merge(crate::modules::purchase::create_router(state.clone()))
        .merge(crate::modules::inventory::create_router(state.clone()))
        .merge(crate::modules::accounting::create_router(state.clone()))
        .merge(crate::modules::manufacturing::create_router(state.clone()))
        .merge(crate::modules::projects::create_router(state))
}
