// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave-slo` portable-coverage gaps.
//!
//! Upstream tracking: nobl9/nobl9-go v0.126.1 — the Go SDK/API client for the
//! Nobl9 SaaS. `cave-slo` ports the genuinely portable *computational* surface
//! (error-budget + burn-rate + multi-window Google-SRE evaluation, composite
//! SLO weighting, objective-window arithmetic, SLI→error-rate conversion, and
//! in-memory status aggregation). These tests target public, implemented cave
//! functions that lacked a direct behavioral test; every expected value is
//! derived from the implementation logic in `src/{engine,models,store}.rs`.

use chrono::Utc;
use uuid::Uuid;

use cave_slo::engine::{composite_slo_compliance, evaluate_multi_window};
use cave_slo::models::{
    MetricType, SloAnnotations, SloIndicator, SloObjective, SloStatus, SLO,
};
use cave_slo::store::SloStore;

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

// ── evaluate_multi_window ───────────────────────────────────────────────────

#[test]
fn multi_window_fires_both_alerts_when_burning_fast() {
    // Target 99.9% → budget_fraction = 0.001.
    // 1h: Ratio good=984/total=1000 → error 1.6% → burn = 0.016 / 0.001 = 16.0.
    //     16.0 >= PAGE_BURN_RATE_THRESHOLD (14.4) → short_window_alert = true,
    //     and 16.0 >= 14.4 → status = Breached.
    // 6h: Ratio good=993/total=1000 → error 0.7% → burn = 0.007 / 0.001 = 7.0.
    //     7.0 >= LONG_WINDOW_BURN_THRESHOLD (6.0) → long_window_alert = true.
    let slo = make_slo(99.9, 30);
    let eval = evaluate_multi_window(
        &slo,
        SloIndicator::Ratio { good: 984, total: 1000 },
        SloIndicator::Ratio { good: 993, total: 1000 },
        SloIndicator::Ratio { good: 993, total: 1000 },
        SloIndicator::Ratio { good: 993, total: 1000 },
    );
    assert!((eval.burn_rate_1h - 16.0).abs() < 1e-6, "br_1h={}", eval.burn_rate_1h);
    assert!((eval.burn_rate_6h - 7.0).abs() < 1e-6, "br_6h={}", eval.burn_rate_6h);
    assert!(eval.short_window_alert);
    assert!(eval.long_window_alert);
    assert_eq!(eval.status, SloStatus::Breached);
    assert_eq!(eval.slo_id, slo.id);
}

#[test]
fn multi_window_no_alerts_when_healthy() {
    // All windows perfect (0% error) → burn rate 0 for each → no alerts,
    // and from_burn_rate(0.0) == Ok.
    let slo = make_slo(99.9, 30);
    let eval = evaluate_multi_window(
        &slo,
        SloIndicator::Ratio { good: 1000, total: 1000 },
        SloIndicator::Ratio { good: 1000, total: 1000 },
        SloIndicator::Ratio { good: 1000, total: 1000 },
        SloIndicator::Ratio { good: 1000, total: 1000 },
    );
    assert!((eval.burn_rate_1h - 0.0).abs() < 1e-9);
    assert!((eval.burn_rate_6h - 0.0).abs() < 1e-9);
    assert!(!eval.short_window_alert);
    assert!(!eval.long_window_alert);
    assert_eq!(eval.status, SloStatus::Ok);
}

// ── composite_slo_compliance ────────────────────────────────────────────────

#[test]
fn composite_weighted_average() {
    // (0.7 * 99.0 + 0.3 * 95.0) / (0.7 + 0.3) = (69.3 + 28.5) / 1.0 = 97.8.
    let objectives = vec![
        SloObjective { name: "avail".into(), target: 99.9, window_days: 30, weight: 0.7 },
        SloObjective { name: "latency".into(), target: 99.0, window_days: 30, weight: 0.3 },
    ];
    let result = composite_slo_compliance(&objectives, &[99.0, 95.0]);
    assert!((result - 97.8).abs() < 1e-9, "result={result}");
}

#[test]
fn composite_empty_objectives_is_zero() {
    let result = composite_slo_compliance(&[], &[]);
    assert!((result - 0.0).abs() < 1e-12);
}

#[test]
fn composite_zero_total_weight_is_zero() {
    let objectives = vec![
        SloObjective { name: "a".into(), target: 99.0, window_days: 30, weight: 0.0 },
        SloObjective { name: "b".into(), target: 99.0, window_days: 30, weight: 0.0 },
    ];
    let result = composite_slo_compliance(&objectives, &[99.0, 95.0]);
    assert!((result - 0.0).abs() < 1e-12);
}

// ── SloObjective window arithmetic ──────────────────────────────────────────

#[test]
fn objective_window_and_budget_minutes() {
    // window_days 30 → 30 * 24 * 60 = 43200 minutes.
    // allowed_bad = 43200 * (1 - 99.9/100) = 43200 * 0.001 = 43.2 minutes.
    let obj = SloObjective {
        name: "obj".into(),
        target: 99.9,
        window_days: 30,
        weight: 1.0,
    };
    assert!((obj.window_minutes() - 43200.0).abs() < 1e-6);
    assert!((obj.allowed_bad_minutes() - 43.2).abs() < 1e-6);
}

// ── SloIndicator::error_rate_pct ────────────────────────────────────────────

#[test]
fn indicator_error_rate_all_arms() {
    // Latency: 0% when p99 < threshold, else 100%.
    assert!((SloIndicator::Latency { p99_ms: 200.0, threshold_ms: 150.0 }.error_rate_pct()
        - 100.0)
        .abs()
        < 1e-9);
    assert!((SloIndicator::Latency { p99_ms: 100.0, threshold_ms: 150.0 }.error_rate_pct()
        - 0.0)
        .abs()
        < 1e-9);
    // Threshold: 0% when value < threshold, else 100% (equal counts as breach).
    assert!((SloIndicator::Threshold { value: 5.0, threshold: 10.0 }.error_rate_pct()
        - 0.0)
        .abs()
        < 1e-9);
    assert!((SloIndicator::Threshold { value: 10.0, threshold: 10.0 }.error_rate_pct()
        - 100.0)
        .abs()
        < 1e-9);
    // Ratio: total==0 guard returns 0; otherwise (total-good)/total*100.
    assert!((SloIndicator::Ratio { good: 0, total: 0 }.error_rate_pct() - 0.0).abs() < 1e-9);
    assert!(
        (SloIndicator::Ratio { good: 990, total: 1000 }.error_rate_pct() - 1.0).abs() < 1e-9
    );
}

// ── SloStore::compute_stats + list_by_status ────────────────────────────────

#[test]
fn store_compute_stats_and_list_by_status() {
    let store = SloStore::new();

    let mut mk = |status: SloStatus, sli: f64| {
        let mut slo = make_slo(99.9, 30);
        slo.status = status;
        slo.current_sli = sli;
        let id = slo.id;
        store.insert(slo);
        id
    };

    // 5 SLOs: one per status. current_sli values sum to
    // 100 + 95 + 80 + 60 + 50 = 385; mean = 385 / 5 = 77.0.
    mk(SloStatus::Ok, 100.0);
    mk(SloStatus::AtRisk, 95.0);
    mk(SloStatus::Breaching, 80.0);
    let breached_id = mk(SloStatus::Breached, 60.0);
    mk(SloStatus::Unknown, 50.0);

    let stats = store.compute_stats();
    assert_eq!(stats.total, 5);
    assert_eq!(stats.ok, 1);
    assert_eq!(stats.at_risk, 1);
    assert_eq!(stats.breaching, 1);
    assert_eq!(stats.breached, 1);
    // Unknown is counted in total but in no status bucket.
    assert!((stats.avg_compliance - 77.0).abs() < 1e-9, "avg={}", stats.avg_compliance);

    let breached = store.list_by_status(SloStatus::Breached);
    assert_eq!(breached.len(), 1);
    assert_eq!(breached[0].id, breached_id);
    assert_eq!(breached[0].status, SloStatus::Breached);
}

// ── SloAnnotations set/get/remove ───────────────────────────────────────────

#[test]
fn annotations_set_get_remove() {
    let mut ann = SloAnnotations::default();
    assert_eq!(ann.get("team"), None);

    ann.set("team", "payments");
    assert_eq!(ann.get("team"), Some("payments"));

    // Overwrite existing key.
    ann.set("team", "checkout");
    assert_eq!(ann.get("team"), Some("checkout"));

    ann.remove("team");
    assert_eq!(ann.get("team"), None);
}
