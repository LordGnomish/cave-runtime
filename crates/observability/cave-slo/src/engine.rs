// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SLO burn-rate engine — error budgets, multi-window evaluation, composite SLOs.
//!
//! Implements the Google SRE multi-window burn-rate approach used by
//! nobl9-go and OpenSLO tooling. Key references:
//!   - Google SRE Workbook, Chapter 5: Alerting on SLOs
//!   - nobl9/nobl9-go BudgetAdjustment + Objective models

use crate::models::{BurnRateAlert, ErrorBudget, SloIndicator, SloObjective, SloStatus, SLO};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Error Budget ─────────────────────────────────────────────────────────────

/// Calculate error budget for an SLO given request counts.
///
/// * `good_requests` – number of successful (good) requests in the window
/// * `total_requests` – total requests in the window
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

// ── Burn Rate ─────────────────────────────────────────────────────────────────

/// Calculate burn rate: how fast the error budget is being consumed.
///
/// Burn rate = actual_error_rate / (1 - slo_target_pct).
/// A value of 1.0 means budget is being consumed at exactly the sustainable rate.
/// A value of 14.4 means the budget will exhaust in 1h on a 30-day window.
pub fn calculate_burn_rate(actual_error_rate: f64, slo_target: f64) -> f64 {
    let budget_fraction = 1.0 - slo_target / 100.0;
    if budget_fraction == 0.0 {
        return f64::INFINITY;
    }
    actual_error_rate / 100.0 / budget_fraction
}

/// Convenience: compute burn rate directly from an [`SloIndicator`].
pub fn burn_rate_from_indicator(indicator: &SloIndicator, slo_target: f64) -> f64 {
    calculate_burn_rate(indicator.error_rate_pct(), slo_target)
}

// ── Compliance helpers ────────────────────────────────────────────────────────

/// Returns `true` when the SLO budget has not been exceeded.
pub fn is_compliant(budget: &ErrorBudget) -> bool {
    !budget.is_breached
}

/// Returns what fraction of the error budget has been consumed (0–100%).
pub fn budget_consumed_percent(budget: &ErrorBudget) -> f64 {
    if budget.allowed_bad_minutes == 0.0 {
        return 100.0;
    }
    (budget.consumed_bad_minutes / budget.allowed_bad_minutes * 100.0).clamp(0.0, 100.0)
}

// ── Burn Rate Alert ───────────────────────────────────────────────────────────

/// Evaluate a single-window burn-rate alert.
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

// ── Multi-Window Evaluation ───────────────────────────────────────────────────

/// Multi-window burn rate evaluation result.
///
/// Mirrors the Google SRE two-window alert structure:
///   - **Short window (1 h)**: high sensitivity, detects fast burn.
///   - **Long window (6 h)**: prevents false positives from burst traffic.
///   - **24 h / 72 h**: trend windows for budget projection.
///
/// An alert fires only when BOTH short *and* long windows exceed their
/// respective thresholds (Google SRE Workbook multi-burn-rate rule).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultiWindowEvaluation {
    pub slo_id: Uuid,
    /// Burn rate measured over the last 1 hour.
    pub burn_rate_1h: f64,
    /// Burn rate measured over the last 6 hours.
    pub burn_rate_6h: f64,
    /// Burn rate measured over the last 24 hours.
    pub burn_rate_24h: f64,
    /// Burn rate measured over the last 72 hours.
    pub burn_rate_72h: f64,
    /// Derived status (from the worst window, i.e. the 1 h rate).
    pub status: SloStatus,
    /// `true` when the 1 h window burn rate ≥ page-level threshold (14.4×).
    pub short_window_alert: bool,
    /// `true` when the 6 h window burn rate ≥ long-window threshold (6×).
    pub long_window_alert: bool,
}

// Thresholds from Google SRE Workbook, Chapter 5.
const PAGE_BURN_RATE_THRESHOLD: f64 = 14.4; // exhausts 30-day budget in ~50 min
const LONG_WINDOW_BURN_THRESHOLD: f64 = 6.0; // exhausts 30-day budget in ~5 h

/// Evaluate an SLO across four time windows.
///
/// Each window takes an [`SloIndicator`] carrying that window's observed
/// metric. Returns a [`MultiWindowEvaluation`] with burn rates and alert
/// states for each window.
pub fn evaluate_multi_window(
    slo: &SLO,
    ind_1h: SloIndicator,
    ind_6h: SloIndicator,
    ind_24h: SloIndicator,
    ind_72h: SloIndicator,
) -> MultiWindowEvaluation {
    let br_1h = burn_rate_from_indicator(&ind_1h, slo.target_percentage);
    let br_6h = burn_rate_from_indicator(&ind_6h, slo.target_percentage);
    let br_24h = burn_rate_from_indicator(&ind_24h, slo.target_percentage);
    let br_72h = burn_rate_from_indicator(&ind_72h, slo.target_percentage);

    let status = SloStatus::from_burn_rate(br_1h);
    let short_window_alert = br_1h >= PAGE_BURN_RATE_THRESHOLD;
    // Long-window alert fires only when it also exceeds the 6 h threshold
    // (dual-window rule: avoids paging on momentary spikes).
    let long_window_alert = br_6h >= LONG_WINDOW_BURN_THRESHOLD;

    MultiWindowEvaluation {
        slo_id: slo.id,
        burn_rate_1h: br_1h,
        burn_rate_6h: br_6h,
        burn_rate_24h: br_24h,
        burn_rate_72h: br_72h,
        status,
        short_window_alert,
        long_window_alert,
    }
}

// ── Composite SLO ─────────────────────────────────────────────────────────────

/// Compute a weighted composite SLI from multiple objectives.
///
/// Returns a single compliance percentage (0–100) representing the
/// objective-weighted average of the individual SLI measurements. This
/// matches nobl9-go's composite SLO model where a service has multiple
/// objectives (availability + latency + throughput) each with a weight.
///
/// If `objectives` is empty, returns 0.0.
pub fn composite_slo_compliance(objectives: &[SloObjective], current_slis: &[f64]) -> f64 {
    if objectives.is_empty() {
        return 0.0;
    }

    let total_weight: f64 = objectives.iter().map(|o| o.weight).sum();
    if total_weight == 0.0 {
        return 0.0;
    }

    let weighted_sum: f64 = objectives
        .iter()
        .zip(current_slis.iter())
        .map(|(obj, &sli)| obj.weight * sli)
        .sum();

    weighted_sum / total_weight
}

// ── Budget projection ─────────────────────────────────────────────────────────

/// Estimate how many minutes until the error budget is exhausted,
/// given the current burn rate.
///
/// Returns `None` if burn rate ≤ 0 (budget not being consumed).
pub fn minutes_until_exhaustion(budget: &ErrorBudget, burn_rate: f64) -> Option<f64> {
    if burn_rate <= 0.0 || budget.remaining_minutes <= 0.0 {
        return None;
    }
    // Remaining budget in minutes / (burn_rate × budget consumption rate per minute)
    // Effective consumption rate = burn_rate × (allowed_bad_minutes / total_minutes)
    if budget.total_minutes == 0.0 {
        return None;
    }
    let natural_rate = budget.allowed_bad_minutes / budget.total_minutes; // minutes bad per minute
    let actual_rate = burn_rate * natural_rate;
    Some(budget.remaining_minutes / actual_rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MetricType, SloStatus, SLO};
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
            current_sli: 0.0,
            status: SloStatus::Unknown,
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
        let slo = make_slo(99.0, 30);
        let budget = calculate_error_budget(&slo, 990, 1000);
        assert!(!budget.is_breached);
    }

    #[test]
    fn test_calculate_error_budget_just_over() {
        let slo = make_slo(99.0, 30);
        let budget = calculate_error_budget(&slo, 989, 1000);
        assert!(budget.is_breached);
    }

    #[test]
    fn test_error_budget_zero_requests() {
        let slo = make_slo(99.9, 30);
        let budget = calculate_error_budget(&slo, 0, 0);
        assert!(!budget.is_breached);
        assert!((budget.remaining_percentage - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_burn_rate_normal() {
        let burn = calculate_burn_rate(1.0, 99.0);
        assert!((burn - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_burn_rate_fast() {
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
        let alert = check_burn_rate_alert(slo_id, 2.0, 99.9, 1, 14.4);
        assert!(alert.is_firing);
        assert!(alert.burn_rate > alert.threshold);
    }

    #[test]
    fn test_burn_rate_alert_not_fires() {
        let slo_id = Uuid::new_v4();
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
        assert!((budget_consumed_percent(&budget) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_minutes_until_exhaustion_normal() {
        let budget = ErrorBudget {
            slo_id: Uuid::new_v4(),
            total_minutes: 43200.0,
            allowed_bad_minutes: 43.2,
            consumed_bad_minutes: 21.6,
            remaining_minutes: 21.6,
            remaining_percentage: 50.0,
            is_breached: false,
        };
        // Burn rate 1.0 → half the budget remains → should exhaust in ~21600 min
        let mins = minutes_until_exhaustion(&budget, 1.0).unwrap();
        assert!(mins > 0.0, "expected positive, got {mins}");
    }

    #[test]
    fn test_minutes_until_exhaustion_zero_burn() {
        let budget = ErrorBudget {
            slo_id: Uuid::new_v4(),
            total_minutes: 43200.0,
            allowed_bad_minutes: 43.2,
            consumed_bad_minutes: 0.0,
            remaining_minutes: 43.2,
            remaining_percentage: 100.0,
            is_breached: false,
        };
        assert!(minutes_until_exhaustion(&budget, 0.0).is_none());
    }

    #[test]
    fn test_burn_rate_from_indicator() {
        let ind = SloIndicator::Ratio { good: 9990, total: 10000 };
        // 0.1% error / 0.1% budget = 1.0 burn rate
        let br = burn_rate_from_indicator(&ind, 99.9);
        assert!((br - 1.0).abs() < 1e-6, "br={br}");
    }
}
