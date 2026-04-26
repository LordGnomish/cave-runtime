//! Gravitee API Monetization — usage-based billing plans, metering, and
//! invoice generation.

use crate::models::*;
use crate::GatewayState;
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct MonetizationStore {
    pub billing_plans: HashMap<Uuid, BillingPlan>,
    /// (consumer_id, api_id) -> current period meter.
    pub meters: HashMap<(Uuid, Uuid), UsageMeter>,
    pub invoices: HashMap<Uuid, Invoice>,
}

impl MonetizationStore {
    pub fn new() -> Self {
        Self {
            billing_plans: HashMap::new(),
            meters: HashMap::new(),
            invoices: HashMap::new(),
        }
    }

    pub fn create_billing_plan(&mut self, req: CreateBillingPlanRequest) -> BillingPlan {
        let plan = BillingPlan {
            id: Uuid::new_v4(),
            name: req.name,
            pricing_model: req.pricing_model,
            base_price: req.base_price.unwrap_or(0.0),
            tiers: req.tiers.unwrap_or_default(),
            billing_period_days: req.billing_period_days.unwrap_or(30),
            created_at: chrono::Utc::now(),
        };
        self.billing_plans.insert(plan.id, plan.clone());
        plan
    }

    pub fn record_usage(&mut self, req: RecordUsageRequest) {
        let now = chrono::Utc::now();
        let meter = self.meters
            .entry((req.consumer_id, req.api_id))
            .or_insert_with(|| UsageMeter {
                consumer_id: req.consumer_id,
                api_id: req.api_id,
                period_start: now,
                period_end: now + chrono::Duration::days(30),
                ..Default::default()
            });
        meter.total_requests += req.requests;
        meter.successful_requests += req.successful;
        meter.failed_requests += req.requests.saturating_sub(req.successful);
        meter.bytes_in += req.bytes_in.unwrap_or(0);
        meter.bytes_out += req.bytes_out.unwrap_or(0);
        if let Some(latency) = req.latency_ms {
            meter.latency_samples.push(latency);
            // Keep samples bounded.
            if meter.latency_samples.len() > 10_000 {
                meter.latency_samples.drain(0..5_000);
            }
        }
    }

    pub fn get_usage(&self, consumer_id: Uuid, api_id: Uuid) -> Option<serde_json::Value> {
        let meter = self.meters.get(&(consumer_id, api_id))?;
        Some(serde_json::json!({
            "consumer_id": meter.consumer_id,
            "api_id": meter.api_id,
            "period_start": meter.period_start,
            "period_end": meter.period_end,
            "total_requests": meter.total_requests,
            "successful_requests": meter.successful_requests,
            "failed_requests": meter.failed_requests,
            "bytes_in": meter.bytes_in,
            "bytes_out": meter.bytes_out,
            "avg_latency_ms": meter.avg_latency_ms(),
            "p99_latency_ms": meter.p99_latency_ms(),
        }))
    }

    pub fn generate_invoice(&mut self, req: GenerateInvoiceRequest) -> Option<Invoice> {
        let plan = self.billing_plans.get(&req.billing_plan_id)?.clone();
        let period_days = req.period_days.unwrap_or(plan.billing_period_days);
        let now = chrono::Utc::now();
        let period_start = now - chrono::Duration::days(period_days as i64);

        // Collect all usage for this consumer across all APIs.
        let total_requests: u64 = self.meters.iter()
            .filter(|((cid, _), _)| *cid == req.consumer_id)
            .map(|(_, m)| m.total_requests)
            .sum();

        let mut lines = Vec::new();

        // Base charge.
        if plan.base_price > 0.0 {
            lines.push(InvoiceLine {
                description: format!("{} — base subscription", plan.name),
                quantity: 1,
                unit_price: plan.base_price,
                amount: plan.base_price,
            });
        }

        // Usage charge.
        let usage_amount = match &plan.pricing_model {
            PricingModel::PerRequest => {
                let unit = plan.base_price / 1000.0;
                total_requests as f64 * unit
            }
            PricingModel::Tiered => {
                let mut amount = 0.0_f64;
                let mut remaining = total_requests;
                for tier in &plan.tiers {
                    let tier_size = tier.to_requests
                        .map(|to| to.saturating_sub(tier.from_requests))
                        .unwrap_or(remaining);
                    let in_tier = remaining.min(tier_size);
                    amount += (in_tier as f64 / 1000.0) * tier.price_per_1k;
                    remaining = remaining.saturating_sub(in_tier);
                    if remaining == 0 { break; }
                }
                amount
            }
            PricingModel::UsageBased => {
                (total_requests as f64 / 1000.0) * plan.tiers.first()
                    .map(|t| t.price_per_1k)
                    .unwrap_or(0.0)
            }
            PricingModel::PerMonth => 0.0,
        };

        if usage_amount > 0.0 {
            lines.push(InvoiceLine {
                description: format!("{} requests metered", total_requests),
                quantity: total_requests,
                unit_price: usage_amount / total_requests.max(1) as f64,
                amount: usage_amount,
            });
        }

        let total = lines.iter().map(|l| l.amount).sum();
        let invoice = Invoice {
            id: Uuid::new_v4(),
            consumer_id: req.consumer_id,
            billing_plan_id: req.billing_plan_id,
            period_start,
            period_end: now,
            lines,
            total_amount: total,
            currency: "USD".to_string(),
            status: InvoiceStatus::Issued,
            generated_at: now,
        };
        self.invoices.insert(invoice.id, invoice.clone());
        Some(invoice)
    }
}

impl Default for MonetizationStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Routes ────────────────────────────────────────────────────────────────────

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/v1/gateway/billing/plans", get(list_billing_plans).post(create_billing_plan))
        .route("/api/v1/gateway/billing/usage", post(record_usage))
        .route("/api/v1/gateway/billing/usage/{consumer_id}/{api_id}", get(get_usage))
        .route("/api/v1/gateway/billing/invoices", post(generate_invoice))
        .route("/api/v1/gateway/billing/invoices/{id}", get(get_invoice))
        .route("/api/v1/gateway/billing/invoices", get(list_invoices))
}

async fn list_billing_plans(State(state): State<Arc<GatewayState>>) -> Json<Vec<BillingPlan>> {
    let store = state.monetization.lock().unwrap();
    Json(store.billing_plans.values().cloned().collect())
}

async fn create_billing_plan(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateBillingPlanRequest>,
) -> Json<BillingPlan> {
    let mut store = state.monetization.lock().unwrap();
    Json(store.create_billing_plan(req))
}

async fn record_usage(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<RecordUsageRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.monetization.lock().unwrap();
    store.record_usage(req);
    Json(serde_json::json!({ "recorded": true }))
}

async fn get_usage(
    State(state): State<Arc<GatewayState>>,
    Path((consumer_id, api_id)): Path<(Uuid, Uuid)>,
) -> Json<serde_json::Value> {
    let store = state.monetization.lock().unwrap();
    match store.get_usage(consumer_id, api_id) {
        Some(v) => Json(v),
        None => Json(serde_json::json!({ "error": "no usage data found" })),
    }
}

async fn generate_invoice(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<GenerateInvoiceRequest>,
) -> Json<serde_json::Value> {
    let mut store = state.monetization.lock().unwrap();
    match store.generate_invoice(req) {
        Some(inv) => Json(serde_json::to_value(inv).unwrap()),
        None => Json(serde_json::json!({ "error": "billing plan not found" })),
    }
}

async fn get_invoice(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<Uuid>,
) -> Json<serde_json::Value> {
    let store = state.monetization.lock().unwrap();
    match store.invoices.get(&id) {
        Some(inv) => Json(serde_json::to_value(inv).unwrap()),
        None => Json(serde_json::json!({ "error": "invoice not found" })),
    }
}

async fn list_invoices(State(state): State<Arc<GatewayState>>) -> Json<Vec<Invoice>> {
    let store = state.monetization.lock().unwrap();
    Json(store.invoices.values().cloned().collect())
}
