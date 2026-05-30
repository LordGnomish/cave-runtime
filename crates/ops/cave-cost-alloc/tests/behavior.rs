// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Behavioral test coverage ported from OpenCost's test corpus (v1.108.0).
// Each test maps to a behavioral unit exercised by upstream Go tests but
// previously uncovered in cave (see docs/audit/tdd/cave-cost-alloc-gaps.md).
// These exercise already-compiled `allocator`/`reporting` functions.

use std::collections::HashMap;

use chrono::{Duration, Utc};
use uuid::Uuid;

use cave_cost_alloc::allocator::{
    allocate_costs, calculate_idle_costs, detect_anomalies, split_shared_costs, RawSpendEntry,
};
use cave_cost_alloc::models::{
    AnomalySeverity, BudgetPeriod, BudgetPolicy, ComplianceStatus, CostCenter, CostLineItem,
    CostReport, InvoiceStatus, ResourceType, SplitStrategy,
};
use cave_cost_alloc::reporting::{
    budget_compliance, forecast_spending, generate_chargeback, generate_showback, unit_economics,
};

// ---- builders -------------------------------------------------------------

fn cost_center(name: &str, team: &str, project: &str, budget: f64) -> CostCenter {
    let now = Utc::now();
    CostCenter {
        id: Uuid::new_v4(),
        name: name.to_string(),
        team: team.to_string(),
        project: project.to_string(),
        department: "eng".to_string(),
        budget_usd: budget,
        owner_email: "owner@example.com".to_string(),
        tags: HashMap::new(),
        created_at: now,
        updated_at: now,
    }
}

fn report(cc_id: Uuid, total: f64) -> CostReport {
    let now = Utc::now();
    CostReport {
        id: Uuid::new_v4(),
        period_start: now - Duration::days(30),
        period_end: now,
        cost_center_id: cc_id,
        environment: "prod".to_string(),
        total_cost_usd: total,
        breakdown: vec![],
        generated_at: now,
    }
}

fn report_with_breakdown(cc_id: Uuid, items: Vec<CostLineItem>) -> CostReport {
    let total: f64 = items.iter().map(|i| i.total_usd).sum();
    let mut r = report(cc_id, total);
    r.breakdown = items;
    r
}

fn line_item(rtype: ResourceType, qty: f64, unit: f64) -> CostLineItem {
    CostLineItem {
        resource_type: rtype,
        description: "item".to_string(),
        quantity: qty,
        unit_price_usd: unit,
        total_usd: qty * unit,
    }
}

fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

// ---- allocate_costs -------------------------------------------------------

// OpenCost: TestLabelConfig_GetExternalAllocationName — tag→cost-center mapping,
// unmatched resources dropped.
#[test]
fn test_allocate_costs_matches_team_project_tags() {
    let cc = cost_center("Platform", "platform", "cave", 10_000.0);
    let centers = vec![cc.clone()];

    let entries = vec![
        RawSpendEntry {
            resource_id: "pod-a".to_string(),
            resource_type: ResourceType::KubernetesPod,
            cost_usd: 12.0,
            tags: tags(&[("team", "platform")]),
        },
        RawSpendEntry {
            resource_id: "pod-b".to_string(),
            resource_type: ResourceType::KubernetesPod,
            cost_usd: 8.0,
            tags: tags(&[("project", "cave")]),
        },
        // untagged → dropped
        RawSpendEntry {
            resource_id: "pod-orphan".to_string(),
            resource_type: ResourceType::KubernetesPod,
            cost_usd: 99.0,
            tags: tags(&[("team", "nobody")]),
        },
    ];

    let allocs = allocate_costs(&centers, &entries);
    assert_eq!(allocs.len(), 2, "only tag-matched resources are allocated");
    assert!(allocs.iter().all(|a| a.cost_center_id == cc.id));
    assert!(allocs.iter().all(|a| a.split_percentage == 100.0));
    assert!(!allocs.iter().any(|a| a.resource_id == "pod-orphan"));
}

// ---- split_shared_costs ---------------------------------------------------

// OpenCost: TestComputeIdleCoefficients — proportional split by usage weight.
#[test]
fn test_split_shared_costs_proportional_by_cpu() {
    let a = cost_center("A", "a", "p", 0.0);
    let b = cost_center("B", "b", "p", 0.0);
    let centers = vec![a.clone(), b.clone()];
    let usage = vec![(a.id, 75.0), (b.id, 25.0)];

    let split = split_shared_costs(100.0, &SplitStrategy::ByCpu, &centers, &usage);
    let got: HashMap<Uuid, f64> = split.into_iter().collect();
    assert!((got[&a.id] - 75.0).abs() < 1e-9);
    assert!((got[&b.id] - 25.0).abs() < 1e-9);
}

// OpenCost: TestAllocation_Share — equal split + zero-usage fallback to equal.
#[test]
fn test_split_shared_costs_equal_and_zero_usage_fallback() {
    let a = cost_center("A", "a", "p", 0.0);
    let b = cost_center("B", "b", "p", 0.0);
    let centers = vec![a.clone(), b.clone()];

    // Equal strategy: even halves regardless of usage.
    let eq = split_shared_costs(100.0, &SplitStrategy::Equal, &centers, &[]);
    assert!(eq.iter().all(|(_, v)| (*v - 50.0).abs() < 1e-9));

    // ByCpu with total usage 0 falls back to equal shares.
    let zero = vec![(a.id, 0.0), (b.id, 0.0)];
    let fb = split_shared_costs(100.0, &SplitStrategy::ByCpu, &centers, &zero);
    assert!(fb.iter().all(|(_, v)| (*v - 50.0).abs() < 1e-9));

    // Empty centers → empty result.
    let empty = split_shared_costs(100.0, &SplitStrategy::Equal, &[], &[]);
    assert!(empty.is_empty());
}

// OpenCost: TestAllocationSet_AggregateBy_SharedCostBreakdown — custom-weight
// normalization against total weight.
#[test]
fn test_split_shared_costs_custom_weights_normalized() {
    let a = cost_center("A", "a", "p", 0.0);
    let b = cost_center("B", "b", "p", 0.0);
    let centers = vec![a.clone(), b.clone()];

    let mut weights = HashMap::new();
    weights.insert(a.id.to_string(), 3.0);
    weights.insert(b.id.to_string(), 1.0);
    let strat = SplitStrategy::ByCustomWeights { weights };

    let split = split_shared_costs(200.0, &strat, &centers, &[]);
    let got: HashMap<Uuid, f64> = split.into_iter().collect();
    // 3/4 and 1/4 of 200.
    assert!((got[&a.id] - 150.0).abs() < 1e-9);
    assert!((got[&b.id] - 50.0).abs() < 1e-9);

    // Zero total weight → empty.
    let zero = SplitStrategy::ByCustomWeights {
        weights: HashMap::new(),
    };
    assert!(split_shared_costs(200.0, &zero, &centers, &[]).is_empty());
}

// ---- calculate_idle_costs -------------------------------------------------

// OpenCost: efficiency/idle tests — below-threshold flagging, monthly waste math,
// recommendation tiering.
#[test]
fn test_calculate_idle_costs_threshold_waste_and_tiers() {
    let util = vec![
        // util 2% (<5 → terminate), hourly $1
        ("vol-dead".to_string(), ResourceType::StorageVolume, None, 2.0, 1.0),
        // util 15% (<20 → downsize)
        ("pod-small".to_string(), ResourceType::KubernetesPod, None, 15.0, 2.0),
        // util 30% (>=20 → review) — still below threshold 50 so flagged idle
        ("node-mid".to_string(), ResourceType::KubernetesNode, None, 30.0, 4.0),
        // util 80% — above threshold, excluded
        ("pod-busy".to_string(), ResourceType::KubernetesPod, None, 80.0, 5.0),
    ];

    let idle = calculate_idle_costs(&util, 50.0);
    assert_eq!(idle.len(), 3, "only sub-threshold resources are idle");

    let dead = idle.iter().find(|r| r.resource_id == "vol-dead").unwrap();
    // 1.0 * 24 * 30 * (1 - 0.02) = 705.6
    assert!((dead.wasted_cost_usd - 705.6).abs() < 1e-6);
    assert!(dead.recommendation.contains("terminate"));

    assert!(idle
        .iter()
        .find(|r| r.resource_id == "pod-small")
        .unwrap()
        .recommendation
        .contains("downsize"));
    assert!(idle
        .iter()
        .find(|r| r.resource_id == "node-mid")
        .unwrap()
        .recommendation
        .contains("review"));
    assert!(!idle.iter().any(|r| r.resource_id == "pod-busy"));
}

// ---- detect_anomalies -----------------------------------------------------

// OpenCost: anomaly/outlier detection — fewer than 2 reports yields nothing,
// deviation threshold suppresses small swings, severity buckets by magnitude.
#[test]
fn test_detect_anomalies_threshold_and_severity() {
    let cc = Uuid::new_v4();

    // Fewer than 2 reports → no anomalies.
    assert!(detect_anomalies(&[report(cc, 100.0)], 50.0).is_empty());

    // High: mean=280, value 1000 → +257% (>100 but <=200 ... actually >200) Critical.
    // Construct a clean High case: values [100,100,100,500] mean=200, 500 → +150% High.
    let high_set = vec![
        report(cc, 100.0),
        report(cc, 100.0),
        report(cc, 100.0),
        report(cc, 500.0),
    ];
    let a = detect_anomalies(&high_set, 60.0);
    assert_eq!(a.len(), 1, "only the 500 report deviates beyond threshold");
    assert!(matches!(a[0].severity, AnomalySeverity::High));
    assert!((a[0].expected_cost_usd - 200.0).abs() < 1e-9);
    assert!((a[0].actual_cost_usd - 500.0).abs() < 1e-9);

    // Critical: values [100,100,100,100,1000] mean=280, 1000 → +257% Critical.
    let crit_set = vec![
        report(cc, 100.0),
        report(cc, 100.0),
        report(cc, 100.0),
        report(cc, 100.0),
        report(cc, 1000.0),
    ];
    let c = detect_anomalies(&crit_set, 80.0);
    assert_eq!(c.len(), 1);
    assert!(matches!(c[0].severity, AnomalySeverity::Critical));
}

// ---- budget_compliance ----------------------------------------------------

// OpenCost / Kubecost budget alerts — Over / Warning / Healthy thresholds,
// cost centers without a policy are omitted.
#[test]
fn test_budget_compliance_status_thresholds() {
    let over = cost_center("Over", "t1", "p1", 0.0);
    let warn = cost_center("Warn", "t2", "p2", 0.0);
    let ok = cost_center("Ok", "t3", "p3", 0.0);
    let no_policy = cost_center("None", "t4", "p4", 0.0);
    let centers = vec![over.clone(), warn.clone(), ok.clone(), no_policy.clone()];

    let now = Utc::now();
    let policy = |cc_id: Uuid, limit: f64, alert: f64| BudgetPolicy {
        id: Uuid::new_v4(),
        cost_center_id: cc_id,
        period: BudgetPeriod::Monthly,
        limit_usd: limit,
        alert_threshold_pct: alert,
        hard_cap: false,
        auto_scale_cap_pct: 100.0,
        created_at: now,
        updated_at: now,
    };
    let policies = vec![
        policy(over.id, 1000.0, 80.0),
        policy(warn.id, 1000.0, 80.0),
        policy(ok.id, 1000.0, 80.0),
    ];
    let reports = vec![
        report(over.id, 1200.0), // 120% → Over
        report(warn.id, 850.0),  // 85% ≥ 80 → Warning
        report(ok.id, 500.0),    // 50% → Healthy
        report(no_policy.id, 9999.0),
    ];

    let entries = budget_compliance(&centers, &policies, &reports);
    assert_eq!(entries.len(), 3, "cost center without a policy is omitted");

    let by_id: HashMap<Uuid, &_> = entries.iter().map(|e| (e.cost_center_id, e)).collect();
    assert!(matches!(by_id[&over.id].status, ComplianceStatus::Over));
    assert!(matches!(by_id[&warn.id].status, ComplianceStatus::Warning));
    assert!(matches!(by_id[&ok.id].status, ComplianceStatus::Healthy));
    assert!((by_id[&over.id].utilization_pct - 120.0).abs() < 1e-9);
}

// ---- forecast_spending ----------------------------------------------------

// OpenCost: prediction/forecast tests — increasing history yields a positive
// linear slope, N forecast points, and confidence tiered by sample size.
#[test]
fn test_forecast_spending_linear_trend_and_confidence() {
    let cc = Uuid::new_v4();
    let increasing: Vec<CostReport> = (1..=6).map(|i| report(cc, 100.0 * i as f64)).collect();

    let model = forecast_spending(cc, &increasing, 3);
    assert!(model.trend_slope > 0.0, "monotonic-increasing history → positive slope");
    assert_eq!(model.forecast_points.len(), 3);
    assert_eq!(model.forecast_months, 3);
    assert!((model.confidence - 0.85).abs() < 1e-9, "6 samples → 0.85 confidence");

    // 3 samples → 0.65
    let three: Vec<CostReport> = (1..=3).map(|i| report(cc, 100.0 * i as f64)).collect();
    assert!((forecast_spending(cc, &three, 2).confidence - 0.65).abs() < 1e-9);

    // 2 samples → 0.40
    let two: Vec<CostReport> = (1..=2).map(|i| report(cc, 100.0 * i as f64)).collect();
    assert!((forecast_spending(cc, &two, 2).confidence - 0.40).abs() < 1e-9);
}

// ---- generate_showback ----------------------------------------------------

// OpenCost showback — actual == showback cost == sum of the center's reports,
// budget-overage tip emitted past 90% of budget.
#[test]
fn test_generate_showback_aggregates_and_tip() {
    let cc = cost_center("Team", "team", "proj", 1000.0);
    let centers = vec![cc.clone()];
    let reports = vec![report(cc.id, 600.0), report(cc.id, 350.0)]; // 950 > 90% of 1000

    let showback = generate_showback(&centers, &reports);
    assert_eq!(showback.len(), 1);
    let s = &showback[0];
    assert!((s.actual_cost_usd - 950.0).abs() < 1e-9);
    assert!((s.showback_cost_usd - 950.0).abs() < 1e-9);
    assert!(
        s.savings_opportunities
            .iter()
            .any(|t| t.contains("90% of budget")),
        "tip emitted when spend exceeds 90% of budget"
    );
}

// ---- generate_chargeback --------------------------------------------------

// OpenCost chargeback — invoice total equals the sum of its line items and the
// invoice opens in Draft status.
#[test]
fn test_generate_chargeback_line_items_and_total() {
    let cc = cost_center("Team", "team", "proj", 1000.0);
    let centers = vec![cc.clone()];
    let reports = vec![report_with_breakdown(
        cc.id,
        vec![
            line_item(ResourceType::Compute, 10.0, 5.0), // 50
            line_item(ResourceType::StorageVolume, 4.0, 2.5), // 10
        ],
    )];

    let invoices = generate_chargeback(&centers, &reports);
    assert_eq!(invoices.len(), 1);
    let inv = &invoices[0];
    assert_eq!(inv.line_items.len(), 2);
    let li_total: f64 = inv.line_items.iter().map(|li| li.total_usd).sum();
    assert!((inv.total_usd - li_total).abs() < 1e-9);
    assert!((inv.total_usd - 60.0).abs() < 1e-9);
    assert!(matches!(inv.status, InvoiceStatus::Draft));
}

// ---- unit_economics -------------------------------------------------------

// OpenCost unit-economics — per-request/user/deployment division with safe
// zero-denominator handling (no NaN / panic).
#[test]
fn test_unit_economics_safe_division() {
    let cc = Uuid::new_v4();
    let reports = vec![report(cc, 600.0), report(cc, 400.0)]; // total 1000

    let ue = unit_economics(&reports, 1000, 50, 0);
    assert!((ue.total_cost_usd - 1000.0).abs() < 1e-9);
    assert!((ue.cost_per_request_usd - 1.0).abs() < 1e-9);
    assert!((ue.cost_per_user_usd - 20.0).abs() < 1e-9);
    // zero deployments → 0.0, not NaN/inf
    assert_eq!(ue.cost_per_deployment_usd, 0.0);
    assert!(ue.cost_per_deployment_usd.is_finite());
}
