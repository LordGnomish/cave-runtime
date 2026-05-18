// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cost calculation model for Kubernetes workloads.

use crate::models::{BudgetAlert, CostAllocation, CostRate, ResourceCost, ResourceUsage};
use std::collections::HashMap;

pub struct CostCalculator;

impl CostCalculator {
    /// Default cost rates based on AWS us-east-1 on-demand pricing (approximate).
    pub fn default_rates() -> CostRate {
        CostRate {
            cpu_per_core_hour: 0.048,
            memory_per_gb_hour: 0.006,
            gpu_per_hour: 0.90,
            storage_per_gb_month: 0.10,
            network_egress_per_gb: 0.09,
            load_balancer_per_hour: 0.008,
        }
    }

    /// Compute the cost for a given usage, rate, and period (in hours).
    pub fn compute_cost(usage: &ResourceUsage, rates: &CostRate, hours: f64) -> ResourceCost {
        let cpu_cost = usage.cpu_cores * rates.cpu_per_core_hour * hours;
        let memory_cost = usage.memory_gb * rates.memory_per_gb_hour * hours;
        let gpu_cost = usage.gpu_count as f64 * rates.gpu_per_hour * hours;
        // Storage is billed per GB-month; convert hours to fraction of month (730 h/month).
        let storage_cost = usage.storage_gb * rates.storage_per_gb_month * (hours / 730.0);
        let network_cost = usage.network_egress_gb * rates.network_egress_per_gb;
        let lb_cost = usage.load_balancers as f64 * rates.load_balancer_per_hour * hours;
        let total_cost = cpu_cost + memory_cost + gpu_cost + storage_cost + network_cost + lb_cost;

        ResourceCost {
            cpu_cost,
            memory_cost,
            gpu_cost,
            storage_cost,
            network_cost,
            lb_cost,
            total_cost,
        }
    }

    /// Compute an efficiency score.
    ///
    /// Uses a weighted average: 60% CPU efficiency + 40% memory efficiency.
    /// Each efficiency is `actual / requested`, capped at 1.0.
    pub fn efficiency_score(requested: &ResourceUsage, actual: &ResourceUsage) -> f32 {
        let cpu_eff = if requested.cpu_cores > 0.0 {
            (actual.cpu_cores / requested.cpu_cores).min(1.0) as f32
        } else {
            1.0
        };
        let mem_eff = if requested.memory_gb > 0.0 {
            (actual.memory_gb / requested.memory_gb).min(1.0) as f32
        } else {
            1.0
        };
        0.6 * cpu_eff + 0.4 * mem_eff
    }

    /// Group allocations by namespace, summing costs.
    pub fn aggregate_by_namespace(
        allocations: &[CostAllocation],
    ) -> HashMap<String, ResourceCost> {
        let mut map: HashMap<String, ResourceCost> = HashMap::new();
        for alloc in allocations {
            let entry = map.entry(alloc.namespace.clone()).or_default();
            *entry = entry.add(&alloc.cost);
        }
        map
    }

    /// Group allocations by team, summing costs.
    pub fn aggregate_by_team(
        allocations: &[CostAllocation],
    ) -> HashMap<String, ResourceCost> {
        let mut map: HashMap<String, ResourceCost> = HashMap::new();
        for alloc in allocations {
            let team = alloc.team.clone().unwrap_or_else(|| "unassigned".into());
            let entry = map.entry(team).or_default();
            *entry = entry.add(&alloc.cost);
        }
        map
    }

    /// Check each budget alert against actual spend calculated from allocations.
    /// Updates `current_spend` and `alert_fired` on each alert.
    pub fn check_budget_alerts(alerts: &mut [BudgetAlert], allocations: &[CostAllocation]) {
        for alert in alerts.iter_mut() {
            let spend: f64 = allocations
                .iter()
                .filter(|a| {
                    let ns_match = alert.namespace.as_deref().is_none_or(|ns| a.namespace == ns);
                    let team_match = alert.team.as_deref().is_none_or(|t| {
                        a.team.as_deref() == Some(t)
                    });
                    let cc_match = alert.cost_center.as_deref().is_none_or(|cc| {
                        a.cost_center.as_deref() == Some(cc)
                    });
                    ns_match && team_match && cc_match
                })
                .map(|a| a.cost.total_cost)
                .sum();

            alert.current_spend = spend;
            alert.alert_fired = spend > alert.threshold_usd;
        }
    }
}
