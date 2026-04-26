use crate::models::{BurnRateAlert, ErrorBudget, SLO};
use uuid::Uuid;

/// Calculate error budget for an SLO given request counts
/// good_requests: number of successful requests in window
/// total_requests: total requests in window
pub fn calculate_error_budget(slo: &SLO, good_requests: u64, total_requests: u64) -> ErrorBudget {
    let total_minutes = slo.window_days as f64 * 24.0 * 60.0;
    let actual_rate = if total_requests == 0 {
        100.0
    } else {
        good_requests as f64 / total_requests as f64 * 100.0
    };
    let allowed_bad_fraction = 1.0 - slo.target_percentage / 100.0;
    let allowed_bad_minutes = total_minutes * allowed_bad_fraction;
    let actual_bad_fraction = 1.0 - actual_rate / 100.0;
    let consumed_bad_minutes = total_minutes * actual_bad_fraction;
    let remaining_minutes = (allowed_bad_minutes - consumed_bad_minutes).max(0.0);
    let remaining_percentage = if allowed_bad_minutes == 0.0 {
        0.0
    } else {
        (remaining_minutes / allowed_bad_minutes * 100.0).clamp(0.0, 100.0)
    };
    let is_breached = consumed_bad_minutes > allowed_bad_minutes;
    ErrorBudget {
        slo_id: slo.id,
        total_minutes,
        allowed_bad_minutes,
        consumed_bad_minutes,
        remaining_minutes,
        remaining_percentage,
        is_breached,
    }
}

/// Calculate burn rate: how fast the error budget is being consumed
/// burn_rate = (actual_error_rate / (1 - slo_target))
/// A burn rate of 1.0 means consuming budget at exactly the rate that depletes it by end of window
pub fn calculate_burn_rate(actual_error_rate: f64, slo_target: f64) -> f64 {
    let budget_fraction = 1.0 - slo_target / 100.0;
    if budget_fraction == 0.0 {
        return f64::INFINITY;
    }
    actual_error_rate / 100.0 / budget_fraction
}

/// Check if SLO is currently compliant
pub fn is_compliant(budget: &ErrorBudget) -> bool {
    !budget.is_breached
}

/// Check if burn rate alert should fire
pub fn check_burn_rate_alert(
    slo_id: Uuid,
    actual_error_rate: f64,
    slo_target: f64,
    window_hours: u32,
    threshold: f64,
) -> BurnRateAlert {
    let burn_rate = calculate_burn_rate(actual_error_rate, slo_target);
    BurnRateAlert {
        slo_id,
        window_hours,
        burn_rate,
        threshold,
        is_firing: burn_rate >= threshold,
    }
}

/// Calculate how much of the window has been consumed
pub fn budget_consumed_percent(budget: &ErrorBudget) -> f64 {
    if budget.allowed_bad_minutes == 0.0 {
        return 100.0;
    }
    (budget.consumed_bad_minutes / budget.allowed_bad_minutes * 100.0).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MetricType, SLO};
    use chrono::Utc;
    use uuid::Uuid;

    fn make_slo(target_percentage: f64, window_days: u32) -> SLO {
        SLO {
            id: Uuid::new_v4(),
            name: "test-slo".to_string(),
            description: "Test SLO".to_string(),
            target_percentage,
            window_days,
            metric_type: MetricType::Availability,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_calculate_error_budget_perfect() {
        let slo = make_slo(99.9, 30);
        let budget = calculate_error_budget(&slo, 1_000_000, 1_000_000);
        assert!(!budget.is_breached);
        assert!((budget.remaining_percentage - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_calculate_error_budget_all_bad() {
        let slo = make_slo(99.9, 30);
        let budget = calculate_error_budget(&slo, 0, 1_000_000);
        assert!(budget.is_breached);
        assert_eq!(budget.remaining_minutes, 0.0);
        assert_eq!(budget.remaining_percentage, 0.0);
    }

    #[test]
    fn test_calculate_error_budget_at_target() {
        // SLO = 99.0%, send exactly 1% bad => at the boundary, not breached
        let slo = make_slo(99.0, 30);
        // 1000 total, 990 good = 1% bad = exactly the budget
        let budget = calculate_error_budget(&slo, 990, 1000);
        assert!(!budget.is_breached);
    }

    #[test]
    fn test_calculate_error_budget_just_over() {
        // SLO = 99.0%, send 1.1% bad => breached
        let slo = make_slo(99.0, 30);
        // 1000 total, 989 good = 1.1% bad = over budget
        let budget = calculate_error_budget(&slo, 989, 1000);
        assert!(budget.is_breached);
    }

    #[test]
    fn test_error_budget_zero_requests() {
        let slo = make_slo(99.9, 30);
        let budget = calculate_error_budget(&slo, 0, 0);
        // 0 requests treated as 100% good => not breached
        assert!(!budget.is_breached);
        assert!((budget.remaining_percentage - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_burn_rate_normal() {
        // SLO = 99.0%, budget_fraction = 0.01, actual error = 1% => burn rate = 1.0
        let burn = calculate_burn_rate(1.0, 99.0);
        assert!((burn - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_burn_rate_fast() {
        // SLO = 99.0%, budget_fraction = 0.01, actual error = 2% => burn rate = 2.0
        let burn = calculate_burn_rate(2.0, 99.0);
        assert!((burn - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_burn_rate_zero_errors() {
        let burn = calculate_burn_rate(0.0, 99.9);
        assert!((burn - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_is_compliant_true() {
        let slo_id = Uuid::new_v4();
        let budget = ErrorBudget {
            slo_id,
            total_minutes: 43200.0,
            allowed_bad_minutes: 43.2,
            consumed_bad_minutes: 20.0,
            remaining_minutes: 23.2,
            remaining_percentage: 53.7,
            is_breached: false,
        };
        assert!(is_compliant(&budget));
    }

    #[test]
    fn test_is_compliant_false() {
        let slo_id = Uuid::new_v4();
        let budget = ErrorBudget {
            slo_id,
            total_minutes: 43200.0,
            allowed_bad_minutes: 43.2,
            consumed_bad_minutes: 100.0,
            remaining_minutes: 0.0,
            remaining_percentage: 0.0,
            is_breached: true,
        };
        assert!(!is_compliant(&budget));
    }

    #[test]
    fn test_burn_rate_alert_fires() {
        let slo_id = Uuid::new_v4();
        // SLO 99.9%, actual error rate 2% => burn rate = 0.02/0.001 = 20.0
        let alert = check_burn_rate_alert(slo_id, 2.0, 99.9, 1, 14.4);
        assert!(alert.is_firing);
        assert!(alert.burn_rate > alert.threshold);
    }

    #[test]
    fn test_burn_rate_alert_not_fires() {
        let slo_id = Uuid::new_v4();
        // SLO 99.9%, actual error rate 0.05% => burn rate = 0.0005/0.001 = 0.5
        let alert = check_burn_rate_alert(slo_id, 0.05, 99.9, 1, 14.4);
        assert!(!alert.is_firing);
        assert!(alert.burn_rate < alert.threshold);
    }

    #[test]
    fn test_budget_consumed_percent() {
        let slo_id = Uuid::new_v4();
        let budget = ErrorBudget {
            slo_id,
            total_minutes: 43200.0,
            allowed_bad_minutes: 100.0,
            consumed_bad_minutes: 50.0,
            remaining_minutes: 50.0,
            remaining_percentage: 50.0,
            is_breached: false,
        };
        let consumed = budget_consumed_percent(&budget);
        assert!((consumed - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_budget_consumed_percent_zero_allowed() {
        let slo_id = Uuid::new_v4();
        let budget = ErrorBudget {
            slo_id,
            total_minutes: 43200.0,
            allowed_bad_minutes: 0.0,
            consumed_bad_minutes: 0.0,
            remaining_minutes: 0.0,
            remaining_percentage: 0.0,
            is_breached: false,
        };
        // When allowed_bad_minutes is 0, returns 100%
        assert!((budget_consumed_percent(&budget) - 100.0).abs() < 1e-9);
    }
}
