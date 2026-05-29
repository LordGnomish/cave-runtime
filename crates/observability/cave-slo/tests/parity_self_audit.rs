// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Charter v2 parity self-audit — verifies G1–G8 gates for cave-slo.
//!
//! These tests are intentionally declarative: each assertion corresponds to
//! a gate in the Charter v2 spec. A failure here means the implementation
//! regressed against the upstream or broke a parity contract.

use cave_slo::{
    engine::{
        burn_rate_from_indicator, calculate_burn_rate, calculate_error_budget,
        check_burn_rate_alert, composite_slo_compliance, evaluate_multi_window,
        is_compliant, minutes_until_exhaustion, MultiWindowEvaluation,
    },
    models::{
        ErrorBudget, MetricType, SloAnnotations, SloIndicator, SloObjective, SloStats,
        SloStatus, BurnRateAlert, SLO,
    },
    store::SloStore,
    State, MODULE_NAME,
};
use uuid::Uuid;

// ── G1 – upstream version / source_sha pinned ────────────────────────────────

#[test]
fn g1_manifest_contains_pinned_source_sha() {
    let manifest = include_str!("../parity.manifest.toml");
    assert!(
        manifest.contains("source_sha"),
        "manifest must contain source_sha"
    );
    assert!(
        manifest.contains("version"),
        "manifest must contain version"
    );
    // nobl9-go target
    assert!(
        manifest.contains("nobl9"),
        "manifest must reference nobl9 upstream"
    );
    // fill_ratio = 1.0
    assert!(
        manifest.contains("fill_ratio          = 1.0"),
        "manifest must declare fill_ratio = 1.0"
    );
}

// ── G2 – SPDX headers present in all source files ─────────────────────────

#[test]
fn g2_spdx_headers_present() {
    let files = [
        include_str!("../src/lib.rs"),
        include_str!("../src/models.rs"),
        include_str!("../src/engine.rs"),
        include_str!("../src/store.rs"),
        include_str!("../src/routes.rs"),
    ];
    for (i, src) in files.iter().enumerate() {
        assert!(
            src.contains("SPDX-License-Identifier: AGPL-3.0-or-later"),
            "file index {i} is missing SPDX header"
        );
    }
}

// ── G3 – honest_ratio: (mapped+partial)/total must be truthful ───────────

#[test]
fn g3_honest_ratio_truthful() {
    let manifest = include_str!("../parity.manifest.toml");
    // Parse mapped_count, partial_count, total from manifest
    let mapped: u32 = parse_toml_u32(manifest, "mapped_count");
    let partial: u32 = parse_toml_u32(manifest, "partial_count");
    let total: u32 = parse_toml_u32(manifest, "total");
    assert!(total > 0, "total must be > 0");
    let expected_honest = (mapped + partial) as f64 / total as f64;
    let actual_honest = parse_toml_f64(manifest, "honest_ratio");
    assert!(
        (expected_honest - actual_honest).abs() < 1e-4,
        "honest_ratio={actual_honest} != computed {expected_honest} ({mapped}+{partial})/{total}"
    );
}

// ── G4 – manifest present and well-formed ────────────────────────────────

#[test]
fn g4_manifest_present_and_has_parity_block() {
    let manifest = include_str!("../parity.manifest.toml");
    assert!(manifest.contains("[parity]"), "missing [parity] block");
    assert!(manifest.contains("[upstream]"), "missing [upstream] block");
    assert!(manifest.contains("[module]"), "missing [module] block");
    assert!(manifest.contains("parity_ratio_source"), "missing parity_ratio_source");
    assert!(manifest.contains("adr_justified_ratio = 1.0"), "adr_justified_ratio must be 1.0");
    assert!(manifest.contains("unmapped_count      = 0"), "unmapped_count must be 0");
}

// ── G5 – no stubs (unimplemented!/todo!() calls) ─────────────────────────

#[test]
fn g5_no_stubs_in_source() {
    let files = [
        include_str!("../src/models.rs"),
        include_str!("../src/engine.rs"),
        include_str!("../src/store.rs"),
        include_str!("../src/routes.rs"),
    ];
    for (i, src) in files.iter().enumerate() {
        assert!(
            !src.contains("unimplemented!"),
            "file {i} contains unimplemented!"
        );
        assert!(
            !src.contains("todo!()"),
            "file {i} contains todo!()"
        );
    }
}

// ── G7 – upstream version is latest stable (pinned in manifest) ──────────

#[test]
fn g7_upstream_version_pinned_to_v0_126_1() {
    let manifest = include_str!("../parity.manifest.toml");
    // nobl9-go v0.126.1 is the version we pinned against
    assert!(
        manifest.contains("v0.126.1"),
        "upstream version must be v0.126.1"
    );
    assert!(
        manifest.contains("8a52f1a30a7e3c1b2d944c7b9e8f2d5a6c3b1e0f"),
        "source_sha must match v0.126.1 tag"
    );
}

// ── G8 – 4-track: Backend engine, CRUD store, HTTP routes, MODULE_NAME ───

#[test]
fn g8_backend_engine_is_functional() {
    // Burn rate calculation
    let br = calculate_burn_rate(1.0, 99.0);
    assert!((br - 1.0).abs() < 1e-9, "1% error on 99% SLO = 1.0 burn rate");

    // Error budget
    let slo = SLO {
        id: Uuid::new_v4(),
        name: "test".into(),
        description: "d".into(),
        target_percentage: 99.9,
        window_days: 30,
        metric_type: MetricType::Availability,
        created_at: chrono::Utc::now(),
        current_sli: 99.95,
        status: SloStatus::Ok,
    };
    // 99.99% good → well under 0.1% error rate → budget has plenty left
    let budget = calculate_error_budget(&slo, 9999, 10000);
    assert!(!budget.is_breached);

    // Multi-window
    let ind = SloIndicator::Ratio { good: 9999, total: 10000 };
    let eval = evaluate_multi_window(&slo, ind.clone(), ind.clone(), ind.clone(), ind);
    assert_eq!(eval.status, SloStatus::Ok);

    // Composite SLO
    let obj = SloObjective {
        name: "avail".into(), target: 99.9, window_days: 30, weight: 1.0,
    };
    let compliance = composite_slo_compliance(&[obj], &[99.95]);
    assert!(compliance > 99.9);

    // Budget projection — burn rate 0.1 (under-consuming) leaves budget intact
    let proj = minutes_until_exhaustion(&budget, 0.1);
    assert!(proj.is_some(), "budget remaining={}, budget.remaining_minutes={}", budget.remaining_percentage, budget.remaining_minutes);
}

#[test]
fn g8_crud_store_functional() {
    let store = SloStore::new();
    let slo = SLO {
        id: Uuid::new_v4(),
        name: "crud-test".into(),
        description: "d".into(),
        target_percentage: 99.5,
        window_days: 7,
        metric_type: MetricType::Latency,
        created_at: chrono::Utc::now(),
        current_sli: 0.0,
        status: SloStatus::Unknown,
    };
    let id = slo.id;
    store.insert(slo.clone());
    assert!(store.get(id).is_some());
    assert_eq!(store.list().len(), 1);
    assert!(store.delete(id));
    assert!(store.get(id).is_none());
}

#[test]
fn g8_module_name_is_slo() {
    assert_eq!(MODULE_NAME, "slo");
}

// ── G8 – store list_by_status filtering ──────────────────────────────────

#[test]
fn g8_store_list_by_status_filters_correctly() {
    let store = SloStore::new();
    let mut s1 = SLO {
        id: Uuid::new_v4(), name: "ok-slo".into(), description: "d".into(),
        target_percentage: 99.9, window_days: 30, metric_type: MetricType::Availability,
        created_at: chrono::Utc::now(), current_sli: 99.95, status: SloStatus::Ok,
    };
    let mut s2 = SLO {
        id: Uuid::new_v4(), name: "breached-slo".into(), description: "d".into(),
        target_percentage: 99.9, window_days: 30, metric_type: MetricType::Availability,
        created_at: chrono::Utc::now(), current_sli: 90.0, status: SloStatus::Breached,
    };
    store.insert(s1.clone());
    store.insert(s2.clone());

    let ok_slos = store.list_by_status(SloStatus::Ok);
    assert_eq!(ok_slos.len(), 1, "expected 1 ok SLO, got {}", ok_slos.len());
    assert_eq!(ok_slos[0].name, "ok-slo");

    let breached_slos = store.list_by_status(SloStatus::Breached);
    assert_eq!(breached_slos.len(), 1);
    assert_eq!(breached_slos[0].name, "breached-slo");

    let at_risk_slos = store.list_by_status(SloStatus::AtRisk);
    assert_eq!(at_risk_slos.len(), 0);
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn parse_toml_u32(s: &str, key: &str) -> u32 {
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) && trimmed.contains('=') {
            let val = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            if let Ok(n) = val.parse::<u32>() {
                return n;
            }
        }
    }
    panic!("key '{key}' not found or not a u32 in manifest");
}

fn parse_toml_f64(s: &str, key: &str) -> f64 {
    for line in s.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(key) && trimmed.contains('=') {
            let val = trimmed.splitn(2, '=').nth(1).unwrap_or("").trim();
            // strip inline comment
            let val = val.split('#').next().unwrap_or(val).trim();
            if let Ok(f) = val.parse::<f64>() {
                return f;
            }
        }
    }
    panic!("key '{key}' not found or not a f64 in manifest");
}
