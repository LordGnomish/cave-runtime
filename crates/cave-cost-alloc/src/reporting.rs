use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{
    BudgetComplianceEntry, BudgetPolicy, ComplianceStatus, CostCenter, CostLineItem, CostReport,
    ForecastModel, ForecastPoint, Invoice, InvoiceLineItem, InvoiceStatus, ResourceType,
    ShowbackReport, UnitEconomics,
};

/// Generate showback (awareness-only) reports per team.
pub fn generate_showback(cost_centers: &[CostCenter], reports: &[CostReport]) -> Vec<ShowbackReport> {
    let now = Utc::now();

    cost_centers
        .iter()
        .map(|cc| {
            let cc_reports: Vec<&CostReport> =
                reports.iter().filter(|r| r.cost_center_id == cc.id).collect();

            let actual_cost: f64 = cc_reports.iter().map(|r| r.total_cost_usd).sum();
            let (period_start, period_end) = period_range(&cc_reports, now);

            ShowbackReport {
                id: Uuid::new_v4(),
                period_start,
                period_end,
                cost_center_id: cc.id,
                team: cc.team.clone(),
                actual_cost_usd: actual_cost,
                showback_cost_usd: actual_cost,
                savings_opportunities: identify_savings(cc, &cc_reports),
                generated_at: now,
            }
        })
        .collect()
}

/// Generate actual chargeback invoices per cost center.
pub fn generate_chargeback(cost_centers: &[CostCenter], reports: &[CostReport]) -> Vec<Invoice> {
    let now = Utc::now();

    cost_centers
        .iter()
        .map(|cc| {
            let cc_reports: Vec<&CostReport> =
                reports.iter().filter(|r| r.cost_center_id == cc.id).collect();

            let line_items: Vec<InvoiceLineItem> = cc_reports
                .iter()
                .flat_map(|r| r.breakdown.iter().map(cost_line_to_invoice_item))
                .collect();

            let total: f64 = line_items.iter().map(|li| li.total_usd).sum();
            let (period_start, period_end) = period_range(&cc_reports, now);

            Invoice {
                id: Uuid::new_v4(),
                cost_center_id: cc.id,
                period_start,
                period_end,
                line_items,
                total_usd: total,
                status: InvoiceStatus::Draft,
                issued_at: None,
                due_date: None,
            }
        })
        .collect()
}

/// Predict the next `months_ahead` months of spending via linear trend.
pub fn forecast_spending(
    cost_center_id: Uuid,
    historical_reports: &[CostReport],
    months_ahead: u32,
) -> ForecastModel {
    let now = Utc::now();

    let costs: Vec<f64> = historical_reports
        .iter()
        .filter(|r| r.cost_center_id == cost_center_id)
        .map(|r| r.total_cost_usd)
        .collect();

    let (slope, intercept) = linear_regression(&costs);
    let n = costs.len() as f64;

    let forecast_points = (1..=months_ahead)
        .map(|i| {
            let t = n + i as f64;
            let predicted = (slope * t + intercept).max(0.0);
            let band = predicted * 0.15;
            ForecastPoint {
                month: format!("T+{i}"),
                predicted_cost_usd: predicted,
                lower_bound_usd: (predicted - band).max(0.0),
                upper_bound_usd: predicted + band,
            }
        })
        .collect();

    let confidence = match costs.len() {
        n if n >= 6 => 0.85,
        n if n >= 3 => 0.65,
        _ => 0.40,
    };

    ForecastModel {
        id: Uuid::new_v4(),
        cost_center_id,
        forecast_months: months_ahead,
        trend_slope: slope,
        trend_intercept: intercept,
        forecast_points,
        confidence,
        generated_at: now,
    }
}

/// Check which cost centers are over, near, or under their budgets.
pub fn budget_compliance(
    cost_centers: &[CostCenter],
    policies: &[BudgetPolicy],
    reports: &[CostReport],
) -> Vec<BudgetComplianceEntry> {
    cost_centers
        .iter()
        .filter_map(|cc| {
            let policy = policies.iter().find(|p| p.cost_center_id == cc.id)?;

            let spend: f64 = reports
                .iter()
                .filter(|r| r.cost_center_id == cc.id)
                .map(|r| r.total_cost_usd)
                .sum();

            let utilization = if policy.limit_usd > 0.0 {
                (spend / policy.limit_usd) * 100.0
            } else {
                0.0
            };

            let status = if utilization > 100.0 {
                ComplianceStatus::Over
            } else if utilization >= policy.alert_threshold_pct {
                ComplianceStatus::Warning
            } else {
                ComplianceStatus::Healthy
            };

            Some(BudgetComplianceEntry {
                cost_center_id: cc.id,
                cost_center_name: cc.name.clone(),
                budget_limit_usd: policy.limit_usd,
                current_spend_usd: spend,
                utilization_pct: utilization,
                status,
            })
        })
        .collect()
}

/// Compute cost per request, per user, and per deployment across all reports.
pub fn unit_economics(
    reports: &[CostReport],
    total_requests: u64,
    total_users: u64,
    total_deployments: u64,
) -> UnitEconomics {
    let now = Utc::now();
    let total_cost: f64 = reports.iter().map(|r| r.total_cost_usd).sum();

    let (period_start, period_end) = {
        let refs: Vec<&CostReport> = reports.iter().collect();
        period_range(&refs, now)
    };

    UnitEconomics {
        period_start,
        period_end,
        cost_per_request_usd: safe_div(total_cost, total_requests as f64),
        cost_per_user_usd: safe_div(total_cost, total_users as f64),
        cost_per_deployment_usd: safe_div(total_cost, total_deployments as f64),
        total_requests,
        total_users,
        total_deployments,
        total_cost_usd: total_cost,
    }
}

// --- helpers ---

fn safe_div(n: f64, d: f64) -> f64 {
    if d == 0.0 { 0.0 } else { n / d }
}

/// Ordinary least-squares linear regression on a time series.
/// Returns `(slope, intercept)` for `y ≈ slope * t + intercept`.
fn linear_regression(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    if n < 2.0 {
        return (0.0, values.first().copied().unwrap_or(0.0));
    }
    let sum_x: f64 = (0..values.len()).map(|i| i as f64).sum();
    let sum_y: f64 = values.iter().sum();
    let sum_xy: f64 = values.iter().enumerate().map(|(i, y)| i as f64 * y).sum();
    let sum_x2: f64 = (0..values.len()).map(|i| (i as f64).powi(2)).sum();
    let denom = n * sum_x2 - sum_x.powi(2);
    if denom == 0.0 {
        return (0.0, sum_y / n);
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;
    (slope, intercept)
}

fn period_range(
    reports: &[&CostReport],
    default: DateTime<Utc>,
) -> (DateTime<Utc>, DateTime<Utc>) {
    let mut sorted = reports.to_vec();
    sorted.sort_by_key(|r| r.period_start);
    match (sorted.first(), sorted.last()) {
        (Some(first), Some(last)) => (first.period_start, last.period_end),
        _ => (default, default),
    }
}

fn identify_savings(cc: &CostCenter, reports: &[&CostReport]) -> Vec<String> {
    let mut tips = Vec::new();
    let total: f64 = reports.iter().map(|r| r.total_cost_usd).sum();

    if total > cc.budget_usd * 0.9 {
        tips.push("Spending exceeds 90% of budget — review resource sizing".to_string());
    }

    let has_idle_storage = reports.iter().any(|r| {
        r.breakdown
            .iter()
            .any(|li| matches!(li.resource_type, ResourceType::StorageVolume) && li.quantity > 0.0)
    });
    if has_idle_storage {
        tips.push("Review unattached storage volumes for deletion".to_string());
    }

    tips
}

fn cost_line_to_invoice_item(li: &CostLineItem) -> InvoiceLineItem {
    InvoiceLineItem {
        description: li.description.clone(),
        resource_type: li.resource_type.clone(),
        quantity: li.quantity,
        unit_price_usd: li.unit_price_usd,
        total_usd: li.total_usd,
    }
}
