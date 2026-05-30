// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for cave-cost.
//!
//! These exercise public cost-math behaviors that port upstream OpenCost v1.108.0
//! (https://github.com/opencost/opencost) semantics: per-resource cost scaling by
//! window duration, allocation aggregation by non-namespace dimensions, showback
//! grouping, trend forecasting/projection, report window bounds, rightsizing
//! confidence tiers, recommendation merge, and budget threshold boundaries.
//!
//! Every assertion checks a concrete value derived from the implementation logic,
//! not a tautology. Where the implementation diverges from its doc-comment (e.g.
//! aggregation never populates `idle_cost`, so efficiency is always 1.0; and
//! `merge_recommendations` only concatenates), the test documents the *actual*
//! shipped behavior.

use cave_cost::allocation::aggregate_costs;
use cave_cost::budget::evaluate_budget;
use cave_cost::calculator::calculate_resource_cost;
use cave_cost::models::{
    AggregateBy, Budget, BudgetStatus, CloudProvider, CostAllocation, PricingConfig,
    RecommendationKind, ReportWindow, ResourceCost, ResourceType, ShowbackType,
};
use cave_cost::recommendations::{merge_recommendations, rightsizing_recommendations};
use cave_cost::reports::{build_showback_report, generate_trend, window_bounds};
use chrono::{Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

const GIB: u64 = 1024 * 1024 * 1024;

fn pricing() -> PricingConfig {
    PricingConfig {
        id: Uuid::new_v4(),
        name: "test".to_string(),
        provider: CloudProvider::Aws,
        cpu_core_hour: 0.048,
        memory_gb_hour: 0.006,
        storage_gb_month: 0.10,
        network_egress_gb: 0.09,
        gpu_core_hour: 2.48,
        custom_rates: HashMap::new(),
        created_at: Utc::now(),
    }
}

/// A ResourceCost where only the *aggregation-relevant* fields matter.
/// cpu/memory/storage/network/total costs are passed explicitly so aggregation
/// sums are deterministic; usage fields are inert here.
fn alloc_input(
    namespace: &str,
    pod: Option<&str>,
    controller: Option<&str>,
    labels: &[(&str, &str)],
    cpu_cost: f64,
    mem_cost: f64,
) -> ResourceCost {
    let labels: HashMap<String, String> = labels
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    ResourceCost {
        id: Uuid::new_v4(),
        resource_type: ResourceType::Pod,
        namespace: namespace.to_string(),
        pod: pod.map(|s| s.to_string()),
        controller: controller.map(|s| s.to_string()),
        controller_kind: None,
        labels,
        annotations: HashMap::new(),
        cpu_cores: 1.0,
        cpu_cores_used: 0.5,
        memory_bytes: GIB,
        memory_bytes_used: GIB / 2,
        storage_bytes: 0,
        network_egress_bytes: 0,
        gpu_cores: 0.0,
        cpu_cost,
        memory_cost: mem_cost,
        storage_cost: 0.0,
        network_cost: 0.0,
        gpu_cost: 0.0,
        total_cost: cpu_cost + mem_cost,
        window_start: Utc::now(),
        window_end: Utc::now(),
    }
}

/// A ResourceCost for the rightsizing path, parametrized by request/usage.
fn rightsizing_input(cpu_req: f64, cpu_used: f64, mem_req: u64, mem_used: u64) -> ResourceCost {
    ResourceCost {
        id: Uuid::new_v4(),
        resource_type: ResourceType::Pod,
        namespace: "default".to_string(),
        pod: Some("pod-a".to_string()),
        controller: None,
        controller_kind: None,
        labels: HashMap::new(),
        annotations: HashMap::new(),
        cpu_cores: cpu_req,
        cpu_cores_used: cpu_used,
        memory_bytes: mem_req,
        memory_bytes_used: mem_used,
        storage_bytes: 0,
        network_egress_bytes: 0,
        gpu_cores: 0.0,
        cpu_cost: 1.0,
        memory_cost: 1.0,
        storage_cost: 0.0,
        network_cost: 0.0,
        gpu_cost: 0.0,
        total_cost: 2.0,
        window_start: Utc::now(),
        window_end: Utc::now(),
    }
}

fn make_allocation(namespace: &str, labels: &[(&str, &str)], total: f64) -> CostAllocation {
    let now = Utc::now();
    let labels: HashMap<String, String> = labels
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    CostAllocation {
        namespace: namespace.to_string(),
        labels,
        controller: None,
        total_cost: total,
        cpu_cost: total * 0.6,
        memory_cost: total * 0.4,
        storage_cost: 0.0,
        network_cost: 0.0,
        idle_cost: 0.0,
        shared_cost: 0.0,
        efficiency: 1.0,
        window_start: now,
        window_end: now,
    }
}

// 1. aggregate_costs by Controller and by Label — non-namespace dimensions.
#[test]
fn test_aggregate_by_controller_and_label() {
    let costs = vec![
        alloc_input("ns1", Some("p1"), Some("web"), &[("app", "web")], 10.0, 0.0),
        alloc_input("ns2", Some("p2"), Some("web"), &[("app", "web")], 5.0, 0.0),
        alloc_input("ns3", Some("p3"), Some("db"), &[("app", "db")], 3.0, 0.0),
        // controller None -> falls back to "__none__" key
        alloc_input("ns4", Some("p4"), None, &[], 7.0, 0.0),
    ];
    let now = Utc::now();

    let by_controller = aggregate_costs(&costs, &AggregateBy::Controller, now, now);
    // 3 groups: "web" (10+5=15), "db" (3), "__none__" (7)
    assert_eq!(by_controller.len(), 3);
    let web = by_controller
        .iter()
        .find(|a| a.controller.as_deref() == Some("web"))
        .expect("web controller group");
    assert!((web.total_cost - 15.0).abs() < 1e-9);
    let none_group = by_controller
        .iter()
        .find(|a| a.controller.is_none())
        .expect("controller-less group keyed __none__");
    assert!((none_group.total_cost - 7.0).abs() < 1e-9);

    let by_label = aggregate_costs(&costs, &AggregateBy::Label, now, now);
    // label keys: "app=web" (10+5=15), "app=db" (3), "" (empty labels -> 7)
    assert_eq!(by_label.len(), 3);
    let web_label = by_label
        .iter()
        .find(|a| a.labels.get("app").map(String::as_str) == Some("web"))
        .expect("app=web label group");
    assert!((web_label.total_cost - 15.0).abs() < 1e-9);
}

// 2. aggregate_costs efficiency rollup. idle_cost is never populated by the
// aggregation loop (entries start at 0.0 and only cpu/mem/storage/net/total are
// summed), so for any group with total_cost > 0 efficiency == (total-0)/total = 1.0.
#[test]
fn test_aggregate_computes_efficiency() {
    let costs = vec![
        alloc_input("prod", Some("p1"), None, &[], 8.0, 2.0),
        alloc_input("prod", Some("p2"), None, &[], 4.0, 1.0),
    ];
    let now = Utc::now();
    let allocs = aggregate_costs(&costs, &AggregateBy::Namespace, now, now);
    assert_eq!(allocs.len(), 1);
    let prod = &allocs[0];
    // summed: total = (8+2)+(4+1) = 15
    assert!((prod.total_cost - 15.0).abs() < 1e-9);
    // efficiency = (total - idle)/total, idle never set => exactly 1.0, in [0,1]
    assert!((prod.efficiency - 1.0).abs() < 1e-9);
    assert!(prod.efficiency >= 0.0 && prod.efficiency <= 1.0);
}

// 3. build_showback_report groups by team label, falling back to "unallocated".
#[test]
fn test_build_showback_report_groups_by_team() {
    let allocs = vec![
        make_allocation("ns-a", &[("team", "platform")], 30.0),
        make_allocation("ns-b", &[], 12.0), // no team label -> "unallocated"
    ];
    let now = Utc::now();
    let report = build_showback_report(
        "q1".to_string(),
        ShowbackType::Chargeback,
        now,
        now,
        &allocs,
    );
    assert_eq!(report.line_items.len(), 2);
    let platform = report
        .line_items
        .iter()
        .find(|l| l.team == "platform")
        .expect("platform line item");
    assert!((platform.cost - 30.0).abs() < 1e-9);
    let unalloc = report
        .line_items
        .iter()
        .find(|l| l.team == "unallocated")
        .expect("unallocated fallback line item");
    assert!((unalloc.cost - 12.0).abs() < 1e-9);
    // total sums the line items
    assert!((report.total_cost - 42.0).abs() < 1e-9);
}

// 4. calculate_resource_cost scales cpu/memory cost linearly with window hours.
#[test]
fn test_calculate_cost_scales_with_window_hours() {
    let p = pricing();
    let start = Utc::now();
    let labels = HashMap::new();
    let annos = HashMap::new();

    let one_h = calculate_resource_cost(
        "ns",
        Some("pod"),
        None,
        None,
        labels.clone(),
        annos.clone(),
        2.0,           // cpu cores
        1.0,
        4 * GIB,       // memory bytes
        GIB,
        0,
        0,
        0.0,
        start,
        start + Duration::hours(1),
        &p,
    );
    let two_h = calculate_resource_cost(
        "ns",
        Some("pod"),
        None,
        None,
        labels,
        annos,
        2.0,
        1.0,
        4 * GIB,
        GIB,
        0,
        0,
        0.0,
        start,
        start + Duration::hours(2),
        &p,
    );

    // exact 1h cpu cost = cores * rate * hours = 2 * 0.048 * 1 = 0.096
    assert!((one_h.cpu_cost - 0.096).abs() < 1e-9);
    // exact 1h memory cost = 4 GiB * 0.006 * 1 = 0.024
    assert!((one_h.memory_cost - 0.024).abs() < 1e-9);
    // doubling the window doubles cpu and memory cost
    assert!((two_h.cpu_cost - one_h.cpu_cost * 2.0).abs() < 1e-9);
    assert!((two_h.memory_cost - one_h.memory_cost * 2.0).abs() < 1e-9);
}

// 5. calculate_resource_cost computes storage / network / gpu components.
#[test]
fn test_calculate_cost_storage_network_gpu() {
    let p = pricing();
    let start = Utc::now();
    let cost = calculate_resource_cost(
        "ns",
        Some("pod"),
        None,
        None,
        HashMap::new(),
        HashMap::new(),
        0.0,         // no cpu
        0.0,
        0,           // no memory
        0,
        10 * GIB,    // storage bytes
        20 * GIB,    // network egress bytes
        1.0,         // gpu cores
        start,
        start + Duration::hours(1),
        &p,
    );
    // storage = 10 GiB * (0.10 / 730) * 1h
    let expected_storage = 10.0 * (0.10 / 730.0) * 1.0;
    assert!((cost.storage_cost - expected_storage).abs() < 1e-9);
    // network = 20 GiB * 0.09 (NOT scaled by hours) = 1.8
    assert!((cost.network_cost - 1.8).abs() < 1e-9);
    // gpu = 1 core * 2.48 * 1h = 2.48
    assert!((cost.gpu_cost - 2.48).abs() < 1e-9);
    // cpu and memory are zero here
    assert_eq!(cost.cpu_cost, 0.0);
    assert_eq!(cost.memory_cost, 0.0);
    // total is the sum of the three components
    let expected_total = expected_storage + 1.8 + 2.48;
    assert!((cost.total_cost - expected_total).abs() < 1e-9);
}

// 6. generate_trend emits 7 forecast points at avg-daily, projected = avg*30.
#[test]
fn test_generate_trend_forecast_and_projection() {
    let now = Utc::now();
    // 4 days of data, mean = (10+20+30+40)/4 = 25.0
    let series = vec![
        (now - Duration::days(3), 10.0),
        (now - Duration::days(2), 20.0),
        (now - Duration::days(1), 30.0),
        (now, 40.0),
    ];
    let trend = generate_trend(series, Some("billing".to_string()));
    assert_eq!(trend.data_points.len(), 4);
    assert_eq!(trend.forecast_points.len(), 7);
    // every forecast point equals the average daily spend (25.0)
    for fp in &trend.forecast_points {
        assert!((fp.cost - 25.0).abs() < 1e-9);
    }
    // projected monthly = avg_daily * 30 = 750.0
    assert!((trend.projected_monthly_cost - 750.0).abs() < 1e-9);
    // mom change = (last-first)/first*100 = (40-10)/10*100 = 300%
    assert!((trend.month_over_month_change - 300.0).abs() < 1e-9);
}

// 7. window_bounds: LastWeek ~7 days, LastMonth ~30 days, Custom honors explicit
// bounds and falls back to a 7-day span when None.
#[test]
fn test_window_bounds_week_month_custom() {
    let (ws, we) = window_bounds(&ReportWindow::LastWeek, None, None);
    assert_eq!((we - ws).num_days(), 7);

    let (ms, me) = window_bounds(&ReportWindow::LastMonth, None, None);
    assert_eq!((me - ms).num_days(), 30);

    // Custom with explicit bounds is honored exactly
    let cs = Utc::now() - Duration::days(3);
    let ce = Utc::now();
    let (got_s, got_e) = window_bounds(&ReportWindow::Custom, Some(cs), Some(ce));
    assert_eq!(got_s, cs);
    assert_eq!(got_e, ce);

    // Custom with no bounds falls back to a 7-day window
    let (fb_s, fb_e) = window_bounds(&ReportWindow::Custom, None, None);
    assert_eq!((fb_e - fb_s).num_days(), 7);
}

// 8. rightsizing confidence tiers: cpu_util < 0.25 -> 0.9, else 0.7.
#[test]
fn test_rightsizing_confidence_high_when_very_low_util() {
    // cpu util = 0.4/4.0 = 0.10 (< LOW_UTILIZATION_THRESHOLD 0.25) -> confidence 0.9
    let very_low = vec![rightsizing_input(4.0, 0.4, 4 * GIB, 4 * GIB)];
    let recs = rightsizing_recommendations(&very_low);
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].kind, RecommendationKind::Rightsizing);
    assert!((recs[0].confidence - 0.9).abs() < 1e-9);

    // cpu util = 1.2/4.0 = 0.30 (>=0.25, <0.50) flagged via cpu<0.50 -> confidence 0.7
    let mid = vec![rightsizing_input(4.0, 1.2, 4 * GIB, 4 * GIB)];
    let recs2 = rightsizing_recommendations(&mid);
    assert_eq!(recs2.len(), 1);
    assert!((recs2[0].confidence - 0.7).abs() < 1e-9);
}

// 9. merge_recommendations concatenates (documents actual behavior: no dedup).
#[test]
fn test_merge_recommendations_concatenates() {
    let a = rightsizing_recommendations(&[rightsizing_input(4.0, 0.4, 4 * GIB, 4 * GIB)]);
    let b = rightsizing_recommendations(&[rightsizing_input(8.0, 0.5, 8 * GIB, 8 * GIB)]);
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);
    let merged = merge_recommendations(a, b);
    // concatenation: 1 + 1 = 2, no deduplication despite the doc-comment
    assert_eq!(merged.len(), 2);
}

// 10. budget status boundaries: exactly threshold% -> Warning, exactly 100% -> Exceeded.
#[test]
fn test_budget_status_at_exact_threshold() {
    // spend == 80% of limit, threshold == 80.0 -> inclusive >= -> Warning
    let mut at_threshold = Budget {
        id: Uuid::new_v4(),
        name: "warn".to_string(),
        namespace: None,
        label_selector: HashMap::new(),
        monthly_limit_usd: 100.0,
        alert_threshold_percent: 80.0,
        alert_trend_percent: None,
        current_spend: 80.0,
        forecasted_spend: 0.0, // keep forecast under limit so only threshold alert fires
        status: BudgetStatus::Ok,
        created_at: Utc::now(),
    };
    let alerts = evaluate_budget(&mut at_threshold);
    assert_eq!(at_threshold.status, BudgetStatus::Warning);
    assert_eq!(alerts.len(), 1);
    assert!((alerts[0].percent_used - 80.0).abs() < 1e-9);

    // spend == 100% of limit -> inclusive >= 100.0 -> Exceeded
    let mut at_limit = Budget {
        id: Uuid::new_v4(),
        name: "exceeded".to_string(),
        namespace: None,
        label_selector: HashMap::new(),
        monthly_limit_usd: 100.0,
        alert_threshold_percent: 80.0,
        alert_trend_percent: None,
        current_spend: 100.0,
        forecasted_spend: 0.0,
        status: BudgetStatus::Ok,
        created_at: Utc::now(),
    };
    let alerts2 = evaluate_budget(&mut at_limit);
    assert_eq!(at_limit.status, BudgetStatus::Exceeded);
    assert!((alerts2[0].percent_used - 100.0).abs() < 1e-9);
}
