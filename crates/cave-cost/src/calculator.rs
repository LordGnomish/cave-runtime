// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::{PricingConfig, ResourceCost, ResourceType};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

/// Calculate cost for a resource given usage data and pricing.
pub fn calculate_resource_cost(
    namespace: &str,
    pod: Option<&str>,
    controller: Option<&str>,
    controller_kind: Option<&str>,
    labels: HashMap<String, String>,
    annotations: HashMap<String, String>,
    cpu_cores: f64,
    cpu_cores_used: f64,
    memory_bytes: u64,
    memory_bytes_used: u64,
    storage_bytes: u64,
    network_egress_bytes: u64,
    gpu_cores: f64,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    pricing: &PricingConfig,
) -> ResourceCost {
    let hours = (window_end - window_start).num_seconds() as f64 / 3600.0;
    let memory_gb = memory_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let storage_gb = storage_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    let network_gb = network_egress_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

    let cpu_cost = cpu_cores * pricing.cpu_core_hour * hours;
    let memory_cost = memory_gb * pricing.memory_gb_hour * hours;
    let storage_cost = storage_gb * (pricing.storage_gb_month / 730.0) * hours;
    let network_cost = network_gb * pricing.network_egress_gb;
    let gpu_cost = gpu_cores * pricing.gpu_core_hour * hours;
    let total_cost = cpu_cost + memory_cost + storage_cost + network_cost + gpu_cost;

    ResourceCost {
        id: Uuid::new_v4(),
        resource_type: if pod.is_some() {
            ResourceType::Pod
        } else {
            ResourceType::Namespace
        },
        namespace: namespace.to_string(),
        pod: pod.map(|s| s.to_string()),
        controller: controller.map(|s| s.to_string()),
        controller_kind: controller_kind.map(|s| s.to_string()),
        labels,
        annotations,
        cpu_cores,
        cpu_cores_used,
        memory_bytes,
        memory_bytes_used,
        storage_bytes,
        network_egress_bytes,
        gpu_cores,
        cpu_cost,
        memory_cost,
        storage_cost,
        network_cost,
        gpu_cost,
        total_cost,
        window_start,
        window_end,
    }
}

/// Calculate CPU efficiency (used/requested), capped at 1.0.
pub fn cpu_efficiency(cpu_cores: f64, cpu_cores_used: f64) -> f64 {
    if cpu_cores == 0.0 {
        return 0.0;
    }
    (cpu_cores_used / cpu_cores).min(1.0)
}

/// Calculate memory efficiency (used/requested), capped at 1.0.
pub fn memory_efficiency(memory_bytes: u64, memory_bytes_used: u64) -> f64 {
    if memory_bytes == 0 {
        return 0.0;
    }
    (memory_bytes_used as f64 / memory_bytes as f64).min(1.0)
}

/// Calculate overall resource efficiency as the average of CPU and memory efficiency.
pub fn overall_efficiency(cpu_cores: f64, cpu_used: f64, mem_bytes: u64, mem_used: u64) -> f64 {
    let cpu_eff = cpu_efficiency(cpu_cores, cpu_used);
    let mem_eff = memory_efficiency(mem_bytes, mem_used);
    (cpu_eff + mem_eff) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CloudProvider;
    use chrono::Duration;

    fn test_pricing() -> PricingConfig {
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

    #[test]
    fn test_calculate_cost_basic() {
        let pricing = test_pricing();
        let now = Utc::now();
        let cost = calculate_resource_cost(
            "default",
            Some("my-pod"),
            None,
            None,
            HashMap::new(),
            HashMap::new(),
            1.0,
            0.5,
            1024 * 1024 * 1024,
            512 * 1024 * 1024,
            0,
            0,
            0.0,
            now,
            now + Duration::hours(1),
            &pricing,
        );
        assert!(cost.cpu_cost > 0.0);
        assert!(cost.memory_cost > 0.0);
        assert!(
            (cost.total_cost
                - (cost.cpu_cost
                    + cost.memory_cost
                    + cost.storage_cost
                    + cost.network_cost
                    + cost.gpu_cost))
                .abs()
                < 1e-10
        );
    }

    #[test]
    fn test_efficiency() {
        assert_eq!(cpu_efficiency(1.0, 0.5), 0.5);
        assert_eq!(cpu_efficiency(0.0, 0.0), 0.0);
        assert_eq!(cpu_efficiency(1.0, 1.5), 1.0); // capped at 1.0
    }

    #[test]
    fn test_memory_efficiency() {
        assert_eq!(memory_efficiency(1024, 512), 0.5);
        assert_eq!(memory_efficiency(0, 0), 0.0);
    }

    #[test]
    fn test_overall_efficiency() {
        let eff = overall_efficiency(2.0, 1.0, 2048, 1024);
        assert!((eff - 0.5).abs() < 1e-10);
    }
}
