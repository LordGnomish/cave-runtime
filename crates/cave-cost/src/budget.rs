// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{AlertType, Budget, BudgetAlert, BudgetStatus};

/// Evaluate a budget against its current spend and forecast, returning any triggered alerts.
/// Also updates the budget's `status` field in place.
pub fn evaluate_budget(budget: &mut Budget) -> Vec<BudgetAlert> {
    let mut alerts = Vec::new();
    let percent_used = if budget.monthly_limit_usd > 0.0 {
        (budget.current_spend / budget.monthly_limit_usd) * 100.0
    } else {
        0.0
    };

    // Update status
    budget.status = if percent_used >= 100.0 {
        BudgetStatus::Exceeded
    } else if percent_used >= budget.alert_threshold_percent {
        BudgetStatus::Warning
    } else {
        BudgetStatus::Ok
    };

    // Threshold / exceeded alert
    if percent_used >= budget.alert_threshold_percent {
        alerts.push(BudgetAlert {
            budget_id: budget.id,
            budget_name: budget.name.clone(),
            alert_type: if percent_used >= 100.0 {
                AlertType::ThresholdExceeded
            } else {
                AlertType::ThresholdExceeded
            },
            current_spend: budget.current_spend,
            limit: budget.monthly_limit_usd,
            percent_used,
            triggered_at: chrono::Utc::now(),
        });
    }

    // Forecast alert
    if budget.forecasted_spend > budget.monthly_limit_usd {
        alerts.push(BudgetAlert {
            budget_id: budget.id,
            budget_name: budget.name.clone(),
            alert_type: AlertType::ForecastExceeded,
            current_spend: budget.current_spend,
            limit: budget.monthly_limit_usd,
            percent_used,
            triggered_at: chrono::Utc::now(),
        });
    }

    // Trend-based alert
    if let Some(trend_threshold) = budget.alert_trend_percent {
        let trend_percent = if budget.monthly_limit_usd > 0.0 {
            ((budget.forecasted_spend - budget.monthly_limit_usd) / budget.monthly_limit_usd)
                * 100.0
        } else {
            0.0
        };
        if trend_percent >= trend_threshold {
            alerts.push(BudgetAlert {
                budget_id: budget.id,
                budget_name: budget.name.clone(),
                alert_type: AlertType::TrendBased,
                current_spend: budget.current_spend,
                limit: budget.monthly_limit_usd,
                percent_used,
                triggered_at: chrono::Utc::now(),
            });
        }
    }

    alerts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_budget(limit: f64, spend: f64, threshold: f64) -> Budget {
        Budget {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            namespace: None,
            label_selector: HashMap::new(),
            monthly_limit_usd: limit,
            alert_threshold_percent: threshold,
            alert_trend_percent: None,
            current_spend: spend,
            forecasted_spend: spend * 1.2,
            status: BudgetStatus::Ok,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_budget_exceeded() {
        let mut b = make_budget(100.0, 110.0, 80.0);
        let alerts = evaluate_budget(&mut b);
        assert_eq!(b.status, BudgetStatus::Exceeded);
        assert!(!alerts.is_empty());
    }

    #[test]
    fn test_budget_warning() {
        let mut b = make_budget(100.0, 85.0, 80.0);
        let alerts = evaluate_budget(&mut b);
        assert_eq!(b.status, BudgetStatus::Warning);
        assert!(!alerts.is_empty());
    }

    #[test]
    fn test_budget_ok() {
        let mut b = make_budget(100.0, 50.0, 80.0);
        let alerts = evaluate_budget(&mut b);
        assert_eq!(b.status, BudgetStatus::Ok);
        // No threshold alert, but forecast (50 * 1.2 = 60) is under 100, so no forecast alert
        let threshold_alerts: Vec<_> = alerts
            .iter()
            .filter(|a| matches!(a.alert_type, AlertType::ThresholdExceeded))
            .collect();
        assert!(threshold_alerts.is_empty());
    }

    #[test]
    fn test_forecast_alert() {
        let mut b = Budget {
            id: Uuid::new_v4(),
            name: "fc-test".to_string(),
            namespace: None,
            label_selector: HashMap::new(),
            monthly_limit_usd: 100.0,
            alert_threshold_percent: 80.0,
            alert_trend_percent: None,
            current_spend: 70.0,
            forecasted_spend: 120.0, // over limit
            status: BudgetStatus::Ok,
            created_at: chrono::Utc::now(),
        };
        let alerts = evaluate_budget(&mut b);
        let forecast_alerts: Vec<_> = alerts
            .iter()
            .filter(|a| matches!(a.alert_type, AlertType::ForecastExceeded))
            .collect();
        assert!(!forecast_alerts.is_empty());
    }

    #[test]
    fn test_trend_alert() {
        let mut b = Budget {
            id: Uuid::new_v4(),
            name: "trend-test".to_string(),
            namespace: None,
            label_selector: HashMap::new(),
            monthly_limit_usd: 100.0,
            alert_threshold_percent: 80.0,
            alert_trend_percent: Some(10.0), // alert if trending >10% over limit
            current_spend: 50.0,
            forecasted_spend: 115.0, // 15% over limit
            status: BudgetStatus::Ok,
            created_at: chrono::Utc::now(),
        };
        let alerts = evaluate_budget(&mut b);
        let trend_alerts: Vec<_> = alerts
            .iter()
            .filter(|a| matches!(a.alert_type, AlertType::TrendBased))
            .collect();
        assert!(!trend_alerts.is_empty());
    }
}
