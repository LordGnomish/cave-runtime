// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token budget tracking — per user/team/project, daily/weekly/monthly limits.

use crate::models::{BudgetPeriod, BudgetScope};
use crate::GatewayState;
use chrono::Utc;
use tracing::{info, warn};

/// Record token consumption against all matching budget records.
pub fn track_usage(state: &GatewayState, scope: BudgetScope, scope_id: &str, tokens: u64) {
    let mut budgets = state.budgets.lock().unwrap();
    for budget in budgets.iter_mut() {
        if budget.scope == scope && budget.scope_id == scope_id {
            budget.current_usage += tokens;
            let used_frac = budget.current_usage as f64 / budget.limit.max(1) as f64;
            info!(
                scope_id,
                tokens,
                total = budget.current_usage,
                limit = budget.limit,
                "Token usage recorded"
            );
            if used_frac >= budget.alert_threshold {
                warn!(
                    scope_id,
                    used_pct = format!("{:.1}%", used_frac * 100.0),
                    "Budget alert threshold reached"
                );
            }
        }
    }
}

/// Return `Err` if the scope has insufficient remaining budget.
pub fn check_budget(
    state: &GatewayState,
    scope: &BudgetScope,
    scope_id: &str,
    tokens_requested: u64,
) -> Result<(), String> {
    let budgets = state.budgets.lock().unwrap();
    for budget in budgets.iter() {
        if budget.scope == *scope && budget.scope_id == scope_id {
            if budget.current_usage + tokens_requested > budget.limit {
                return Err(format!(
                    "Budget exceeded for {scope_id}: {}/{} tokens used",
                    budget.current_usage, budget.limit
                ));
            }
        }
    }
    Ok(())
}

/// Forecast the date/time when the budget will be exhausted at the current usage rate.
/// Returns `None` if there is no matching budget or usage rate is zero.
pub fn forecast_usage(
    state: &GatewayState,
    scope: &BudgetScope,
    scope_id: &str,
) -> Option<chrono::DateTime<Utc>> {
    let budgets = state.budgets.lock().unwrap();
    let budget = budgets
        .iter()
        .find(|b| b.scope == *scope && b.scope_id == scope_id)?;

    if budget.current_usage == 0 {
        return None;
    }

    let elapsed_secs = Utc::now()
        .signed_duration_since(budget.period_start)
        .num_seconds()
        .max(1);

    let usage_rate = budget.current_usage as f64 / elapsed_secs as f64; // tokens/sec
    let remaining = budget.limit.saturating_sub(budget.current_usage) as f64;

    if usage_rate <= 0.0 {
        return None;
    }

    let secs_until_exhaustion = (remaining / usage_rate) as i64;
    Some(Utc::now() + chrono::Duration::seconds(secs_until_exhaustion))
}

/// Build a usage report for the given scope/scope_id filters (both optional).
pub fn generate_report(
    state: &GatewayState,
    scope: Option<&BudgetScope>,
    scope_id: Option<&str>,
) -> serde_json::Value {
    let budgets = state.budgets.lock().unwrap();

    let rows: Vec<serde_json::Value> = budgets
        .iter()
        .filter(|b| {
            scope.map_or(true, |s| b.scope == *s) && scope_id.map_or(true, |id| b.scope_id == id)
        })
        .map(|b| {
            let usage_pct = if b.limit > 0 {
                b.current_usage as f64 / b.limit as f64 * 100.0
            } else {
                0.0
            };
            let period_label = match b.period {
                BudgetPeriod::Daily => "daily",
                BudgetPeriod::Weekly => "weekly",
                BudgetPeriod::Monthly => "monthly",
            };
            let scope_label = match b.scope {
                BudgetScope::Team => "team",
                BudgetScope::Project => "project",
                BudgetScope::User => "user",
            };
            serde_json::json!({
                "id": b.id,
                "scope": scope_label,
                "scope_id": b.scope_id,
                "period": period_label,
                "limit": b.limit,
                "current_usage": b.current_usage,
                "usage_percent": format!("{usage_pct:.2}"),
                "alert_threshold": b.alert_threshold,
                "period_start": b.period_start,
            })
        })
        .collect();

    serde_json::json!({
        "generated_at": Utc::now(),
        "total_budgets": rows.len(),
        "budgets": rows,
    })
}
