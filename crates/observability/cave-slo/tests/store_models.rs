// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Tests for extended SLO models, store, and annotation support — written FIRST (TDD).

use cave_slo::models::{
    MetricType, SLO, SloIndicator, SloObjective, SloStatus, SloStats, SloAnnotations,
};
use cave_slo::store::SloStore;
use uuid::Uuid;

fn make_slo(name: &str, target: f64) -> SLO {
    SLO {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: format!("{} description", name),
        target_percentage: target,
        window_days: 30,
        metric_type: MetricType::Availability,
        created_at: chrono::Utc::now(),
        current_sli: 0.0,
        status: SloStatus::Unknown,
    }
}

// ── SloIndicator ────────────────────────────────────────────────────────────

#[test]
fn test_slo_indicator_ratio_serde() {
    let ind = SloIndicator::Ratio { good: 980, total: 1000 };
    let json = serde_json::to_string(&ind).unwrap();
    let back: SloIndicator = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SloIndicator::Ratio { good: 980, total: 1000 }));
}

#[test]
fn test_slo_indicator_latency_serde() {
    let ind = SloIndicator::Latency { p99_ms: 250.0, threshold_ms: 300.0 };
    let json = serde_json::to_string(&ind).unwrap();
    let back: SloIndicator = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SloIndicator::Latency { p99_ms: _, threshold_ms: _ }));
}

#[test]
fn test_slo_indicator_threshold_serde() {
    let ind = SloIndicator::Threshold { value: 0.05, threshold: 0.10 };
    let json = serde_json::to_string(&ind).unwrap();
    let back: SloIndicator = serde_json::from_str(&json).unwrap();
    assert!(matches!(back, SloIndicator::Threshold { .. }));
}

#[test]
fn test_slo_indicator_error_rate() {
    let ind = SloIndicator::Ratio { good: 990, total: 1000 };
    let rate = ind.error_rate_pct();
    assert!((rate - 1.0).abs() < 1e-9, "expected 1.0 got {rate}");
}

#[test]
fn test_slo_indicator_error_rate_latency() {
    // p99 < threshold → 0% error
    let ind = SloIndicator::Latency { p99_ms: 100.0, threshold_ms: 300.0 };
    let rate = ind.error_rate_pct();
    assert!((rate - 0.0).abs() < 1e-9);
}

#[test]
fn test_slo_indicator_error_rate_latency_exceeded() {
    // p99 >= threshold → 100% error
    let ind = SloIndicator::Latency { p99_ms: 350.0, threshold_ms: 300.0 };
    let rate = ind.error_rate_pct();
    assert!((rate - 100.0).abs() < 1e-9);
}

#[test]
fn test_slo_indicator_zero_total() {
    let ind = SloIndicator::Ratio { good: 0, total: 0 };
    let rate = ind.error_rate_pct();
    // Zero total → treat as 0% error (SLA unaffected)
    assert!((rate - 0.0).abs() < 1e-9);
}

// ── SloObjective ────────────────────────────────────────────────────────────

#[test]
fn test_slo_objective_serde() {
    let obj = SloObjective {
        name: "default".to_string(),
        target: 99.9,
        window_days: 30,
        weight: 1.0,
    };
    let json = serde_json::to_string(&obj).unwrap();
    let back: SloObjective = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "default");
    assert!((back.target - 99.9).abs() < 1e-9);
}

#[test]
fn test_slo_objective_budget_minutes() {
    let obj = SloObjective { name: "t".to_string(), target: 99.9, window_days: 30, weight: 1.0 };
    // 30 days = 43200 minutes total; 0.1% bad = 43.2 minutes
    let budget = obj.allowed_bad_minutes();
    assert!((budget - 43.2).abs() < 0.01, "expected ~43.2, got {budget}");
}

// ── SloStatus ───────────────────────────────────────────────────────────────

#[test]
fn test_slo_status_serde() {
    for (st, expected) in [
        (SloStatus::Ok, "\"ok\""),
        (SloStatus::AtRisk, "\"at_risk\""),
        (SloStatus::Breaching, "\"breaching\""),
        (SloStatus::Breached, "\"breached\""),
        (SloStatus::Unknown, "\"unknown\""),
    ] {
        let s = serde_json::to_string(&st).unwrap();
        assert_eq!(s, expected, "status {s} != {expected}");
    }
}

#[test]
fn test_slo_status_from_burn_rate() {
    assert_eq!(SloStatus::from_burn_rate(0.5), SloStatus::Ok);
    assert_eq!(SloStatus::from_burn_rate(3.0), SloStatus::AtRisk);
    assert_eq!(SloStatus::from_burn_rate(8.0), SloStatus::Breaching);
    assert_eq!(SloStatus::from_burn_rate(20.0), SloStatus::Breached);
}

// ── SloAnnotations ──────────────────────────────────────────────────────────

#[test]
fn test_annotations_roundtrip() {
    let mut ann = SloAnnotations::default();
    ann.set("team", "platform");
    ann.set("env", "production");
    let json = serde_json::to_string(&ann).unwrap();
    let back: SloAnnotations = serde_json::from_str(&json).unwrap();
    assert_eq!(back.get("team"), Some("platform"));
    assert_eq!(back.get("env"), Some("production"));
    assert_eq!(back.get("missing"), None);
}

// ── SloStats ────────────────────────────────────────────────────────────────

#[test]
fn test_slo_stats_default() {
    let stats = SloStats::default();
    assert_eq!(stats.total, 0);
    assert_eq!(stats.ok, 0);
    assert_eq!(stats.at_risk, 0);
    assert_eq!(stats.breaching, 0);
    assert_eq!(stats.breached, 0);
    assert!((stats.avg_compliance - 0.0).abs() < 1e-9);
}

// ── SloStore ─────────────────────────────────────────────────────────────────

#[test]
fn test_store_insert_get_roundtrip() {
    let store = SloStore::new();
    let slo = make_slo("test", 99.9);
    let id = slo.id;
    store.insert(slo.clone());
    let got = store.get(id).expect("should find inserted SLO");
    assert_eq!(got.name, "test");
}

#[test]
fn test_store_get_missing_returns_none() {
    let store = SloStore::new();
    assert!(store.get(Uuid::new_v4()).is_none());
}

#[test]
fn test_store_list_empty() {
    let store = SloStore::new();
    assert!(store.list().is_empty());
}

#[test]
fn test_store_list_multiple() {
    let store = SloStore::new();
    store.insert(make_slo("slo1", 99.0));
    store.insert(make_slo("slo2", 99.9));
    let list = store.list();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_store_update_existing() {
    let store = SloStore::new();
    let mut slo = make_slo("original", 99.0);
    let id = slo.id;
    store.insert(slo.clone());
    slo.name = "updated".to_string();
    assert!(store.update(slo));
    assert_eq!(store.get(id).unwrap().name, "updated");
}

#[test]
fn test_store_update_nonexistent_returns_false() {
    let store = SloStore::new();
    let slo = make_slo("phantom", 99.0);
    assert!(!store.update(slo));
}

#[test]
fn test_store_delete_existing() {
    let store = SloStore::new();
    let slo = make_slo("deletable", 99.9);
    let id = slo.id;
    store.insert(slo);
    assert!(store.delete(id));
    assert!(store.get(id).is_none());
}

#[test]
fn test_store_delete_nonexistent_returns_false() {
    let store = SloStore::new();
    assert!(!store.delete(Uuid::new_v4()));
}

#[test]
fn test_store_compute_stats_ok() {
    let store = SloStore::new();
    // Insert SLOs with status derived from current_sli
    let mut slo = make_slo("healthy", 99.0);
    slo.current_sli = 99.5;
    slo.status = SloStatus::Ok;
    store.insert(slo);
    let stats = store.compute_stats();
    assert_eq!(stats.total, 1);
    assert_eq!(stats.ok, 1);
    assert!((stats.avg_compliance - 99.5).abs() < 1e-9);
}

#[test]
fn test_store_compute_stats_mixed() {
    let store = SloStore::new();
    let mut s1 = make_slo("s1", 99.0);
    s1.current_sli = 99.5;
    s1.status = SloStatus::Ok;
    let mut s2 = make_slo("s2", 99.9);
    s2.current_sli = 95.0;
    s2.status = SloStatus::Breached;
    store.insert(s1);
    store.insert(s2);
    let stats = store.compute_stats();
    assert_eq!(stats.total, 2);
    assert_eq!(stats.ok, 1);
    assert_eq!(stats.breached, 1);
    assert!((stats.avg_compliance - (99.5 + 95.0) / 2.0).abs() < 1e-9);
}
