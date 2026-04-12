//! CAVE Cost Allocation — Kubernetes cost visibility, showback, and chargeback.
//!
//! Replaces: Kubecost, OpenCost
//! Cost model based on CPU/memory/GPU/storage/network usage with recommendations.

pub mod cost_model;
pub mod models;
pub mod recommendations;
pub mod routes;
pub mod store;

use axum::Router;
use std::sync::Arc;

pub use store::CostAllocStore;

pub struct CostAllocState {
    pub store: Arc<CostAllocStore>,
}

impl Default for CostAllocState {
    fn default() -> Self {
        Self {
            store: Arc::new(CostAllocStore::new()),
        }
    }
}

pub fn router(state: Arc<CostAllocState>) -> Router {
    routes::create_router(state.store.clone())
}

pub const MODULE_NAME: &str = "cost-alloc";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost_model::CostCalculator;
    use crate::models::*;
    use crate::recommendations::RecommendationEngine;
    use chrono::{Duration, Utc};
    use std::collections::HashMap;
    use uuid::Uuid;

    // ── Cost calculation tests ────────────────────────────────────────────────

    #[test]
    fn cost_calculation_cpu_only() {
        let rates = CostCalculator::default_rates();
        let usage = ResourceUsage {
            cpu_cores: 2.0,
            ..Default::default()
        };
        let cost = CostCalculator::compute_cost(&usage, &rates, 1.0); // 1 hour
        let expected = 2.0 * rates.cpu_per_core_hour;
        assert!((cost.cpu_cost - expected).abs() < 1e-10);
        assert_eq!(cost.memory_cost, 0.0);
        assert_eq!(cost.total_cost, expected);
    }

    #[test]
    fn cost_calculation_memory_only() {
        let rates = CostCalculator::default_rates();
        let usage = ResourceUsage {
            memory_gb: 4.0,
            ..Default::default()
        };
        let cost = CostCalculator::compute_cost(&usage, &rates, 1.0);
        let expected = 4.0 * rates.memory_per_gb_hour;
        assert!((cost.memory_cost - expected).abs() < 1e-10);
        assert_eq!(cost.cpu_cost, 0.0);
        assert_eq!(cost.total_cost, expected);
    }

    #[test]
    fn cost_calculation_multi_resource() {
        let rates = CostCalculator::default_rates();
        let usage = ResourceUsage {
            cpu_cores: 4.0,
            memory_gb: 8.0,
            gpu_count: 1,
            storage_gb: 0.0,
            network_egress_gb: 0.0,
            load_balancers: 0,
        };
        let cost = CostCalculator::compute_cost(&usage, &rates, 1.0);
        let expected_total =
            4.0 * rates.cpu_per_core_hour + 8.0 * rates.memory_per_gb_hour + rates.gpu_per_hour;
        assert!((cost.total_cost - expected_total).abs() < 1e-9);
    }

    #[test]
    fn cost_calculation_with_known_values() {
        let rates = CostRate {
            cpu_per_core_hour: 0.048,
            memory_per_gb_hour: 0.006,
            gpu_per_hour: 0.90,
            storage_per_gb_month: 0.10,
            network_egress_per_gb: 0.09,
            load_balancer_per_hour: 0.008,
        };
        let usage = ResourceUsage {
            cpu_cores: 1.0,
            memory_gb: 2.0,
            ..Default::default()
        };
        let cost = CostCalculator::compute_cost(&usage, &rates, 1.0);
        // 1 core * 0.048 + 2 GB * 0.006 = 0.048 + 0.012 = 0.060
        assert!((cost.total_cost - 0.060).abs() < 1e-10);
    }

    // ── Efficiency score tests ────────────────────────────────────────────────

    #[test]
    fn efficiency_perfect_utilization() {
        let usage = ResourceUsage { cpu_cores: 2.0, memory_gb: 4.0, ..Default::default() };
        let score = CostCalculator::efficiency_score(&usage, &usage);
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn efficiency_50_percent_cpu_utilization() {
        let requested = ResourceUsage { cpu_cores: 2.0, memory_gb: 4.0, ..Default::default() };
        let actual = ResourceUsage { cpu_cores: 1.0, memory_gb: 4.0, ..Default::default() };
        let score = CostCalculator::efficiency_score(&requested, &actual);
        // 0.6 * 0.5 + 0.4 * 1.0 = 0.30 + 0.40 = 0.70
        assert!((score - 0.70).abs() < 0.001);
    }

    #[test]
    fn efficiency_low_cpu_and_memory() {
        let requested = ResourceUsage { cpu_cores: 4.0, memory_gb: 8.0, ..Default::default() };
        let actual = ResourceUsage { cpu_cores: 0.4, memory_gb: 1.6, ..Default::default() };
        let score = CostCalculator::efficiency_score(&requested, &actual);
        // cpu_eff = 0.1, mem_eff = 0.2
        // 0.6 * 0.1 + 0.4 * 0.2 = 0.06 + 0.08 = 0.14
        assert!((score - 0.14).abs() < 0.001);
    }

    #[test]
    fn efficiency_capped_at_1() {
        let requested = ResourceUsage { cpu_cores: 1.0, memory_gb: 1.0, ..Default::default() };
        // Actual exceeds requested (burst)
        let actual = ResourceUsage { cpu_cores: 2.0, memory_gb: 2.0, ..Default::default() };
        let score = CostCalculator::efficiency_score(&requested, &actual);
        assert!((score - 1.0).abs() < 0.001);
    }

    // ── Budget alert tests ────────────────────────────────────────────────────

    #[test]
    fn budget_alert_not_fired_below_threshold() {
        let mut alert = make_budget_alert("prod", 100.0);
        let allocations = vec![make_alloc("prod", "team-a", 50.0)];
        CostCalculator::check_budget_alerts(std::slice::from_mut(&mut alert), &allocations);
        assert!(!alert.alert_fired);
        assert!((alert.current_spend - 50.0).abs() < 0.01);
    }

    #[test]
    fn budget_alert_fired_above_threshold() {
        let mut alert = make_budget_alert("prod", 100.0);
        let allocations = vec![
            make_alloc("prod", "team-a", 60.0),
            make_alloc("prod", "team-b", 80.0),
        ];
        CostCalculator::check_budget_alerts(std::slice::from_mut(&mut alert), &allocations);
        assert!(alert.alert_fired);
        assert!((alert.current_spend - 140.0).abs() < 0.01);
    }

    #[test]
    fn budget_alert_filters_by_namespace() {
        let mut alert = make_budget_alert("prod", 100.0);
        let allocations = vec![
            make_alloc("prod", "team-a", 40.0),
            make_alloc("staging", "team-a", 200.0), // different namespace — excluded
        ];
        CostCalculator::check_budget_alerts(std::slice::from_mut(&mut alert), &allocations);
        assert!(!alert.alert_fired);
        assert!((alert.current_spend - 40.0).abs() < 0.01);
    }

    // ── Aggregate tests ───────────────────────────────────────────────────────

    #[test]
    fn aggregate_by_namespace() {
        let allocations = vec![
            make_alloc("ns-a", "team-x", 30.0),
            make_alloc("ns-a", "team-y", 20.0),
            make_alloc("ns-b", "team-x", 50.0),
        ];
        let by_ns = CostCalculator::aggregate_by_namespace(&allocations);
        assert!((by_ns["ns-a"].total_cost - 50.0).abs() < 0.01);
        assert!((by_ns["ns-b"].total_cost - 50.0).abs() < 0.01);
    }

    #[test]
    fn aggregate_by_team() {
        let allocations = vec![
            make_alloc("ns-a", "team-x", 30.0),
            make_alloc("ns-b", "team-x", 20.0),
            make_alloc("ns-c", "team-y", 10.0),
        ];
        let by_team = CostCalculator::aggregate_by_team(&allocations);
        assert!((by_team["team-x"].total_cost - 50.0).abs() < 0.01);
        assert!((by_team["team-y"].total_cost - 10.0).abs() < 0.01);
    }

    // ── Recommendation tests ──────────────────────────────────────────────────

    #[test]
    fn recommendation_for_low_cpu_efficiency() {
        let rates = CostCalculator::default_rates();
        let alloc = make_alloc_with_efficiency("ns-a", "deploy-x", 0.20); // below 0.3
        let recs = RecommendationEngine::analyze(&[alloc], &rates);
        let has_cpu_rec = recs
            .iter()
            .any(|r| r.recommendation_type == RecommendationType::RightSizeCpu);
        assert!(has_cpu_rec, "Should recommend right-sizing CPU for low efficiency");
    }

    #[test]
    fn recommendation_for_low_memory_efficiency() {
        let rates = CostCalculator::default_rates();
        let alloc = make_alloc_with_efficiency("ns-a", "deploy-x", 0.25); // below 0.4
        let recs = RecommendationEngine::analyze(&[alloc], &rates);
        let has_mem_rec = recs
            .iter()
            .any(|r| r.recommendation_type == RecommendationType::RightSizeMemory);
        assert!(has_mem_rec, "Should recommend right-sizing memory for low efficiency");
    }

    #[test]
    fn no_recommendation_for_high_efficiency() {
        let rates = CostCalculator::default_rates();
        let now = Utc::now();
        let alloc = CostAllocation {
            id: Uuid::new_v4(),
            namespace: "ns-prod".into(),
            deployment: Some("frontend".into()),
            pod: None,
            container: None,
            labels: HashMap::new(),
            team: None,
            cost_center: None,
            usage: ResourceUsage { cpu_cores: 2.0, memory_gb: 4.0, ..Default::default() },
            cost: ResourceCost { cpu_cost: 0.1, memory_cost: 0.05, total_cost: 0.15, ..Default::default() },
            efficiency_score: 0.9, // high efficiency
            period_start: now - Duration::hours(1),
            period_end: now,
        };
        let recs = RecommendationEngine::analyze(&[alloc], &rates);
        let right_size_recs: Vec<_> = recs
            .iter()
            .filter(|r| {
                matches!(
                    r.recommendation_type,
                    RecommendationType::RightSizeCpu | RecommendationType::RightSizeMemory
                )
            })
            .collect();
        assert!(right_size_recs.is_empty(), "No right-size recs for high-efficiency workload");
    }

    // ── Cloud cost ingestion tests ────────────────────────────────────────────

    #[test]
    fn cloud_cost_ingestion_and_retrieval() {
        let store = CostAllocStore::new();
        let now = Utc::now();
        store.add_cloud_cost(AddCloudCostRequest {
            provider: CloudProvider::Aws,
            account_id: "123456789".into(),
            service: "EC2".into(),
            region: "us-east-1".into(),
            resource_id: Some("i-abc123".into()),
            tags: None,
            cost_usd: 45.60,
            usage_quantity: 720.0,
            usage_unit: "Hrs".into(),
            period_start: now - Duration::days(30),
            period_end: now,
        });
        let all = store.list_cloud_costs(None);
        assert_eq!(all.len(), 1);
        assert!((all[0].cost_usd - 45.60).abs() < 0.001);
    }

    #[test]
    fn cloud_cost_filtered_by_provider() {
        let store = CostAllocStore::new();
        let now = Utc::now();
        let base_req = AddCloudCostRequest {
            account_id: "acc1".into(),
            service: "Compute".into(),
            region: "us-east-1".into(),
            resource_id: None,
            tags: None,
            cost_usd: 10.0,
            usage_quantity: 1.0,
            usage_unit: "Hrs".into(),
            period_start: now - Duration::hours(1),
            period_end: now,
            provider: CloudProvider::Aws,
        };
        store.add_cloud_cost(base_req.clone());
        store.add_cloud_cost(AddCloudCostRequest { provider: CloudProvider::Gcp, ..base_req });

        let aws_only = store.list_cloud_costs(Some(CloudProvider::Aws));
        assert_eq!(aws_only.len(), 1);
        assert_eq!(aws_only[0].provider, CloudProvider::Aws);

        let all = store.list_cloud_costs(None);
        assert_eq!(all.len(), 2);
    }

    // ── Showback tests ────────────────────────────────────────────────────────

    #[test]
    fn showback_grouped_by_namespace() {
        let store = CostAllocStore::new();
        let now = Utc::now();
        store.add_allocation(make_alloc_req("ns-a", None, None, 10.0, now));
        store.add_allocation(make_alloc_req("ns-a", None, None, 20.0, now));
        store.add_allocation(make_alloc_req("ns-b", None, None, 30.0, now));

        let report = store.generate_showback_report("namespace");
        assert_eq!(report.line_items.len(), 2);
        let ns_a = report.line_items.iter().find(|i| i.group == "ns-a");
        assert!(ns_a.is_some());
        // ns-a total should be ~30, ns-b ~30 (cost derived from usage * rates * hours)
        // We just check relative ordering and non-zero
        assert!(ns_a.unwrap().cost.total_cost > 0.0);
        assert!((report.line_items[0].percentage + report.line_items[1].percentage - 100.0).abs() < 0.1);
    }

    // ── Chargeback tests ──────────────────────────────────────────────────────

    #[test]
    fn chargeback_allocation_percentages_sum_to_100() {
        let store = CostAllocStore::new();
        let now = Utc::now();
        store.add_allocation(make_alloc_req("ns-a", Some("team-a"), Some("cc-100"), 10.0, now));
        store.add_allocation(make_alloc_req("ns-b", Some("team-b"), Some("cc-200"), 20.0, now));

        let report = store.generate_chargeback_report();
        let total_pct: f64 = report.line_items.iter().map(|i| i.allocation_pct).sum();
        assert!((total_pct - 100.0).abs() < 0.1, "Allocation percentages should sum to 100%");
    }

    #[test]
    fn chargeback_groups_by_cost_center() {
        let store = CostAllocStore::new();
        let now = Utc::now();
        store.add_allocation(make_alloc_req("ns-a", Some("team-a"), Some("cc-eng"), 10.0, now));
        store.add_allocation(make_alloc_req("ns-b", Some("team-a"), Some("cc-eng"), 10.0, now));
        store.add_allocation(make_alloc_req("ns-c", Some("team-b"), Some("cc-ops"), 10.0, now));

        let report = store.generate_chargeback_report();
        assert_eq!(report.line_items.len(), 2);
        let cc_eng = report.line_items.iter().find(|i| i.cost_center == "cc-eng");
        assert!(cc_eng.is_some());
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn make_budget_alert(namespace: &str, threshold: f64) -> BudgetAlert {
        BudgetAlert {
            id: Uuid::new_v4(),
            name: format!("{namespace}-budget"),
            namespace: Some(namespace.into()),
            team: None,
            cost_center: None,
            threshold_usd: threshold,
            period: BudgetPeriod::Monthly,
            current_spend: 0.0,
            alert_fired: false,
            created_at: Utc::now(),
        }
    }

    fn make_alloc(namespace: &str, team: &str, cost: f64) -> CostAllocation {
        let now = Utc::now();
        CostAllocation {
            id: Uuid::new_v4(),
            namespace: namespace.into(),
            deployment: None,
            pod: None,
            container: None,
            labels: HashMap::new(),
            team: Some(team.into()),
            cost_center: None,
            usage: ResourceUsage::default(),
            cost: ResourceCost { total_cost: cost, ..Default::default() },
            efficiency_score: 0.8,
            period_start: now - Duration::hours(1),
            period_end: now,
        }
    }

    fn make_alloc_with_efficiency(
        namespace: &str,
        deployment: &str,
        efficiency: f32,
    ) -> CostAllocation {
        let now = Utc::now();
        CostAllocation {
            id: Uuid::new_v4(),
            namespace: namespace.into(),
            deployment: Some(deployment.into()),
            pod: None,
            container: None,
            labels: HashMap::new(),
            team: None,
            cost_center: None,
            usage: ResourceUsage { cpu_cores: 2.0, memory_gb: 4.0, ..Default::default() },
            cost: ResourceCost {
                cpu_cost: 5.0,
                memory_cost: 2.0,
                total_cost: 7.0,
                ..Default::default()
            },
            efficiency_score: efficiency,
            period_start: now - Duration::hours(24),
            period_end: now,
        }
    }

    fn make_alloc_req(
        namespace: &str,
        team: Option<&str>,
        cost_center: Option<&str>,
        cpu_cores: f64,
        now: chrono::DateTime<chrono::Utc>,
    ) -> CreateAllocationRequest {
        CreateAllocationRequest {
            namespace: namespace.into(),
            deployment: None,
            pod: None,
            container: None,
            labels: None,
            team: team.map(Into::into),
            cost_center: cost_center.map(Into::into),
            usage: ResourceUsage { cpu_cores, memory_gb: cpu_cores * 2.0, ..Default::default() },
            requested_usage: None,
            period_start: now - Duration::hours(1),
            period_end: now,
        }
    }
}
