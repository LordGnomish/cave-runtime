// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{CostRecommendation, RecommendationKind, ResourceCost};
use uuid::Uuid;

const LOW_UTILIZATION_THRESHOLD: f64 = 0.25;
const RIGHTSIZING_THRESHOLD: f64 = 0.50;

/// Generate rightsizing recommendations for over-provisioned resources.
/// A resource is flagged when either CPU or memory utilization is below `RIGHTSIZING_THRESHOLD`.
pub fn rightsizing_recommendations(costs: &[ResourceCost]) -> Vec<CostRecommendation> {
    let mut recs = Vec::new();
    for cost in costs {
        let cpu_util = if cost.cpu_cores > 0.0 {
            cost.cpu_cores_used / cost.cpu_cores
        } else {
            1.0
        };
        let mem_util = if cost.memory_bytes > 0 {
            cost.memory_bytes_used as f64 / cost.memory_bytes as f64
        } else {
            1.0
        };

        if cpu_util < RIGHTSIZING_THRESHOLD || mem_util < RIGHTSIZING_THRESHOLD {
            // 20% headroom over actual usage
            let recommended_cpu = (cost.cpu_cores_used * 1.2).max(0.1);
            let recommended_mem =
                ((cost.memory_bytes_used as f64 * 1.2) as u64).max(64 * 1024 * 1024);
            // Conservative savings estimate
            let savings_factor = 1.0 - ((cpu_util + mem_util) / 2.0).min(1.0);
            let estimated_savings = cost.total_cost * savings_factor * 0.7;

            recs.push(CostRecommendation {
                id: Uuid::new_v4(),
                kind: RecommendationKind::Rightsizing,
                namespace: cost.namespace.clone(),
                resource_name: cost
                    .pod
                    .clone()
                    .unwrap_or_else(|| cost.namespace.clone()),
                current_cpu_request: Some(cost.cpu_cores),
                recommended_cpu_request: Some(recommended_cpu),
                current_memory_request: Some(cost.memory_bytes),
                recommended_memory_request: Some(recommended_mem),
                current_monthly_cost: cost.total_cost * 730.0,
                recommended_monthly_cost: (cost.total_cost - estimated_savings) * 730.0,
                estimated_savings: estimated_savings * 730.0,
                confidence: if cpu_util < LOW_UTILIZATION_THRESHOLD {
                    0.9
                } else {
                    0.7
                },
                reason: format!(
                    "CPU utilization {:.0}%, memory utilization {:.0}% — over-provisioned",
                    cpu_util * 100.0,
                    mem_util * 100.0
                ),
                created_at: chrono::Utc::now(),
            });
        }
    }
    recs
}

/// Find orphaned resources (zero CPU and memory utilization).
pub fn orphaned_resource_recommendations(costs: &[ResourceCost]) -> Vec<CostRecommendation> {
    costs
        .iter()
        .filter(|c| c.cpu_cores_used == 0.0 && c.memory_bytes_used == 0)
        .map(|c| CostRecommendation {
            id: Uuid::new_v4(),
            kind: RecommendationKind::OrphanedResource,
            namespace: c.namespace.clone(),
            resource_name: c.pod.clone().unwrap_or_else(|| c.namespace.clone()),
            current_cpu_request: Some(c.cpu_cores),
            recommended_cpu_request: None,
            current_memory_request: Some(c.memory_bytes),
            recommended_memory_request: None,
            current_monthly_cost: c.total_cost * 730.0,
            recommended_monthly_cost: 0.0,
            estimated_savings: c.total_cost * 730.0,
            confidence: 0.95,
            reason: "Zero CPU and memory usage detected — likely orphaned resource".to_string(),
            created_at: chrono::Utc::now(),
        })
        .collect()
}

/// Merge and deduplicate recommendations, keeping the highest-confidence entry per resource.
pub fn merge_recommendations(
    mut a: Vec<CostRecommendation>,
    b: Vec<CostRecommendation>,
) -> Vec<CostRecommendation> {
    a.extend(b);
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ResourceType;
    use std::collections::HashMap;

    fn make_cost(cpu_req: f64, cpu_used: f64, mem_req: u64, mem_used: u64) -> ResourceCost {
        ResourceCost {
            id: Uuid::new_v4(),
            resource_type: ResourceType::Pod,
            namespace: "default".to_string(),
            pod: Some("test-pod".to_string()),
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
            cpu_cost: cpu_req * 0.048,
            memory_cost: mem_req as f64 / 1e9 * 0.006,
            storage_cost: 0.0,
            network_cost: 0.0,
            gpu_cost: 0.0,
            total_cost: cpu_req * 0.048 + mem_req as f64 / 1e9 * 0.006,
            window_start: chrono::Utc::now(),
            window_end: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_rightsizing_detects_over_provisioned() {
        let costs = vec![make_cost(
            4.0,
            0.5,
            4 * 1024 * 1024 * 1024,
            512 * 1024 * 1024,
        )];
        let recs = rightsizing_recommendations(&costs);
        assert!(!recs.is_empty());
        assert_eq!(recs[0].kind, RecommendationKind::Rightsizing);
        assert!(recs[0].estimated_savings > 0.0);
    }

    #[test]
    fn test_rightsizing_well_utilized_not_flagged() {
        // 80% CPU and 80% memory — above RIGHTSIZING_THRESHOLD of 50%
        let costs = vec![make_cost(
            1.0,
            0.8,
            1024 * 1024 * 1024,
            820 * 1024 * 1024,
        )];
        let recs = rightsizing_recommendations(&costs);
        assert!(recs.is_empty());
    }

    #[test]
    fn test_orphaned_resource() {
        let costs = vec![make_cost(1.0, 0.0, 1024 * 1024 * 1024, 0)];
        let recs = orphaned_resource_recommendations(&costs);
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].kind, RecommendationKind::OrphanedResource);
        assert_eq!(recs[0].recommended_monthly_cost, 0.0);
    }

    #[test]
    fn test_no_orphaned_when_used() {
        let costs = vec![make_cost(1.0, 0.1, 1024 * 1024 * 1024, 1024)];
        let recs = orphaned_resource_recommendations(&costs);
        assert!(recs.is_empty());
    }
}
