// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for multi-window burn rate evaluation + composite SLO logic.
//! Written FIRST per TDD (these will fail until the implementation lands).

use cave_slo::engine::{
    calculate_burn_rate, calculate_error_budget, MultiWindowEvaluation,
    evaluate_multi_window, composite_slo_compliance,
};
use cave_slo::models::{MetricType, SloIndicator, SloObjective, SloStatus, SLO};
use chrono::Utc;
use uuid::Uuid;

fn slo_99_9() -> SLO {
    SLO {
        id: Uuid::new_v4(),
        name: "test".to_string(),
        description: "test slo".to_string(),
        target_percentage: 99.9,
        window_days: 30,
        metric_type: MetricType::Availability,
        created_at: Utc::now(),
        current_sli: 0.0,
        status: SloStatus::Unknown,
    }
}

// ── MultiWindowEvaluation struct ─────────────────────────────────────────────

#[test]
fn test_multi_window_evaluation_serde() {
    let ev = MultiWindowEvaluation {
        slo_id: Uuid::new_v4(),
        burn_rate_1h: 1.2,
        burn_rate_6h: 0.8,
        burn_rate_24h: 0.6,
        burn_rate_72h: 0.5,
        status: SloStatus::Ok,
        short_window_alert: false,
        long_window_alert: false,
    };
    let json = serde_json::to_string(&ev).unwrap();
    let back: MultiWindowEvaluation = serde_json::from_str(&json).unwrap();
    assert!((back.burn_rate_1h - 1.2).abs() < 1e-9);
    assert_eq!(back.status, SloStatus::Ok);
}

// ── evaluate_multi_window ────────────────────────────────────────────────────

#[test]
fn test_evaluate_multi_window_ok() {
    let slo = slo_99_9();
    // All windows: very low error rate → burn rate < 1
    let ev = evaluate_multi_window(
        &slo,
        SloIndicator::Ratio { good: 9999, total: 10000 },  // 0.01% error
        SloIndicator::Ratio { good: 9999, total: 10000 },
        SloIndicator::Ratio { good: 9999, total: 10000 },
        SloIndicator::Ratio { good: 9999, total: 10000 },
    );
    assert_eq!(ev.status, SloStatus::Ok, "status={:?}", ev.status);
    assert!(!ev.short_window_alert);
    assert!(!ev.long_window_alert);
    assert!(ev.burn_rate_1h < 2.0, "burn_rate_1h={}", ev.burn_rate_1h);
}

#[test]
fn test_evaluate_multi_window_breached() {
    let slo = slo_99_9();
    // 2% error rate on 99.9% SLO → burn rate = 0.02/0.001 = 20.0
    let ind = SloIndicator::Ratio { good: 980, total: 1000 };
    let ev = evaluate_multi_window(&slo, ind.clone(), ind.clone(), ind.clone(), ind);
    assert_eq!(ev.status, SloStatus::Breached, "status={:?}", ev.status);
    assert!(ev.short_window_alert);
    assert!(ev.long_window_alert);
    assert!(ev.burn_rate_1h >= 14.4, "burn_rate_1h={}", ev.burn_rate_1h);
}

#[test]
fn test_evaluate_multi_window_at_risk() {
    let slo = slo_99_9();
    // 0.3% error on 99.9% SLO → burn rate = 0.003/0.001 = 3.0 (AtRisk range)
    let ind = SloIndicator::Ratio { good: 997, total: 1000 };
    let ev = evaluate_multi_window(&slo, ind.clone(), ind.clone(), ind.clone(), ind);
    assert_eq!(ev.status, SloStatus::AtRisk, "status={:?} burn={}", ev.status, ev.burn_rate_1h);
    assert!(!ev.short_window_alert);  // threshold is 14.4 for page-level
}

#[test]
fn test_evaluate_multi_window_slo_id_preserved() {
    let slo = slo_99_9();
    let id = slo.id;
    let ind = SloIndicator::Ratio { good: 1000, total: 1000 };
    let ev = evaluate_multi_window(&slo, ind.clone(), ind.clone(), ind.clone(), ind);
    assert_eq!(ev.slo_id, id);
}

// ── composite_slo_compliance ──────────────────────────────────────────────────

#[test]
fn test_composite_slo_compliance_single_objective() {
    let obj = SloObjective {
        name: "primary".to_string(),
        target: 99.9,
        window_days: 30,
        weight: 1.0,
    };
    // current_sli = 99.95 — within target
    let compliance = composite_slo_compliance(&[obj], &[99.95]);
    assert!(compliance > 99.9, "compliance={compliance}");
}

#[test]
fn test_composite_slo_compliance_weighted() {
    let objs = vec![
        SloObjective { name: "avail".to_string(), target: 99.9, window_days: 30, weight: 0.7 },
        SloObjective { name: "latency".to_string(), target: 95.0, window_days: 30, weight: 0.3 },
    ];
    let slis = [99.95_f64, 96.0_f64];
    let compliance = composite_slo_compliance(&objs, &slis);
    // Weighted: 0.7*99.95 + 0.3*96.0 = 69.965 + 28.8 = 98.765
    let expected = 0.7 * 99.95 + 0.3 * 96.0;
    assert!((compliance - expected).abs() < 1e-6, "expected {expected} got {compliance}");
}

#[test]
fn test_composite_slo_compliance_empty() {
    let compliance = composite_slo_compliance(&[], &[]);
    assert!((compliance - 0.0).abs() < 1e-9);
}

// ── calculate_burn_rate edge cases ───────────────────────────────────────────

#[test]
fn test_burn_rate_exact_budget_exhaustion() {
    // Exactly at budget → burn rate = 1.0
    // SLO=99.9%, error=0.1% → 0.001/0.001 = 1.0
    let burn = calculate_burn_rate(0.1, 99.9);
    assert!((burn - 1.0).abs() < 1e-9, "expected 1.0 got {burn}");
}

#[test]
fn test_burn_rate_14x() {
    // 14.4× burn on 99.9% SLO → alert fires immediately
    // error = 1.44% → 0.0144/0.001 = 14.4
    let burn = calculate_burn_rate(1.44, 99.9);
    assert!((burn - 14.4).abs() < 0.001, "expected ~14.4 got {burn}");
}

// ── SloIndicator → error_rate → burn_rate pipeline ───────────────────────────

#[test]
fn test_indicator_to_burn_rate_pipeline() {
    let ind = SloIndicator::Ratio { good: 9856, total: 10000 };
    let error_rate = ind.error_rate_pct(); // 1.44%
    assert!((error_rate - 1.44).abs() < 0.001);
    let burn = calculate_burn_rate(error_rate, 99.9);
    assert!((burn - 14.4).abs() < 0.1, "burn={burn}");
}
