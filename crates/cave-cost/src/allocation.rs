// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::{AggregateBy, CostAllocation, ResourceCost};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Aggregate resource costs by the given dimension.
pub fn aggregate_costs(
    costs: &[ResourceCost],
    by: &AggregateBy,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Vec<CostAllocation> {
    let mut groups: HashMap<String, CostAllocation> = HashMap::new();

    for cost in costs {
        let key = match by {
            AggregateBy::Namespace => cost.namespace.clone(),
            AggregateBy::Controller => cost
                .controller
                .clone()
                .unwrap_or_else(|| "__none__".to_string()),
            AggregateBy::Node => "node".to_string(),
            AggregateBy::Pod => cost
                .pod
                .clone()
                .unwrap_or_else(|| cost.namespace.clone()),
            AggregateBy::Label => cost
                .labels
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(","),
            AggregateBy::Annotation => cost
                .annotations
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(","),
        };

        let entry = groups.entry(key).or_insert_with(|| CostAllocation {
            namespace: cost.namespace.clone(),
            labels: cost.labels.clone(),
            controller: cost.controller.clone(),
            total_cost: 0.0,
            cpu_cost: 0.0,
            memory_cost: 0.0,
            storage_cost: 0.0,
            network_cost: 0.0,
            idle_cost: 0.0,
            shared_cost: 0.0,
            efficiency: 0.0,
            window_start,
            window_end,
        });

        entry.cpu_cost += cost.cpu_cost;
        entry.memory_cost += cost.memory_cost;
        entry.storage_cost += cost.storage_cost;
        entry.network_cost += cost.network_cost;
        entry.total_cost += cost.total_cost;
    }

    // Calculate efficiency for each group
    for alloc in groups.values_mut() {
        if alloc.total_cost > 0.0 {
            let non_idle = alloc.total_cost - alloc.idle_cost;
            alloc.efficiency = (non_idle / alloc.total_cost).clamp(0.0, 1.0);
        }
    }

    groups.into_values().collect()
}

/// Calculate idle cost: allocated minus actually used (never negative).
pub fn calculate_idle_cost(allocated: f64, used: f64) -> f64 {
    (allocated - used).max(0.0)
}

/// Distribute shared costs proportionally by total_cost across allocations.
pub fn distribute_shared_costs(allocations: &mut Vec<CostAllocation>, shared_cost: f64) {
    let total = allocations.iter().map(|a| a.total_cost).sum::<f64>();
    if total == 0.0 {
        return;
    }
    for alloc in allocations.iter_mut() {
        let share = (alloc.total_cost / total) * shared_cost;
        alloc.shared_cost += share;
        alloc.total_cost += share;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ResourceType;
    use uuid::Uuid;

    fn make_cost(ns: &str, cpu_cost: f64, mem_cost: f64) -> ResourceCost {
        ResourceCost {
            id: Uuid::new_v4(),
            resource_type: ResourceType::Namespace,
            namespace: ns.to_string(),
            pod: None,
            controller: None,
            controller_kind: None,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            cpu_cores: 1.0,
            cpu_cores_used: 0.5,
            memory_bytes: 1024 * 1024 * 1024,
            memory_bytes_used: 512 * 1024 * 1024,
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

    #[test]
    fn test_aggregate_by_namespace() {
        let costs = vec![
            make_cost("production", 10.0, 5.0),
            make_cost("staging", 2.0, 1.0),
            make_cost("production", 3.0, 1.0),
        ];
        let now = Utc::now();
        let allocations = aggregate_costs(&costs, &AggregateBy::Namespace, now, now);
        let prod = allocations
            .iter()
            .find(|a| a.namespace == "production")
            .unwrap();
        assert!((prod.total_cost - 19.0).abs() < 0.001);
    }

    #[test]
    fn test_idle_cost() {
        assert_eq!(calculate_idle_cost(10.0, 6.0), 4.0);
        assert_eq!(calculate_idle_cost(5.0, 7.0), 0.0); // no negative idle
    }

    #[test]
    fn test_distribute_shared_costs() {
        let now = Utc::now();
        let mut allocations = vec![
            CostAllocation {
                namespace: "a".to_string(),
                labels: HashMap::new(),
                controller: None,
                total_cost: 10.0,
                cpu_cost: 10.0,
                memory_cost: 0.0,
                storage_cost: 0.0,
                network_cost: 0.0,
                idle_cost: 0.0,
                shared_cost: 0.0,
                efficiency: 1.0,
                window_start: now,
                window_end: now,
            },
            CostAllocation {
                namespace: "b".to_string(),
                labels: HashMap::new(),
                controller: None,
                total_cost: 10.0,
                cpu_cost: 10.0,
                memory_cost: 0.0,
                storage_cost: 0.0,
                network_cost: 0.0,
                idle_cost: 0.0,
                shared_cost: 0.0,
                efficiency: 1.0,
                window_start: now,
                window_end: now,
            },
        ];
        distribute_shared_costs(&mut allocations, 4.0);
        assert!((allocations[0].shared_cost - 2.0).abs() < 1e-10);
        assert!((allocations[1].shared_cost - 2.0).abs() < 1e-10);
    }
}
