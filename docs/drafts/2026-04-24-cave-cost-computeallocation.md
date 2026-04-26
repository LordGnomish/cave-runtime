---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/costmodel.go
upstream_fn: ComputeAllocation
status: draft
tier: 1
created_at: 2026-04-24T18:13:37.301800+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/costmodel.go` → `ComputeAllocation`

## Failing test

```rust
#[tokio::test]
async fn test_compute_allocation() {
    use cave_cost::{ComputeAllocation, CostData, ResourceData};
    use std::collections::HashMap;

    // Create a simple cost data scenario: 1 node running for 1 hour with 2 CPUs and 4Gi RAM
    let node_cpu_cost = 0.05; // $/hour
    let node_ram_cost = 0.01; // $/Gi-hour
    let node_cpu = 2.0;
    let node_ram_gi = 4.0;
    let node_hours = 1.0;

    let mut resource_data = HashMap::new();
    resource_data.insert(
        "node1".to_string(),
        ResourceData {
            cpu_cost_per_hour: node_cpu_cost,
            ram_cost_per_gi_hour: node_ram_cost,
            cpu_cores: node_cpu,
            ram_gi: node_ram_gi,
            start_time: chrono::Utc::now() - chrono::Duration::hours(1),
            end_time: chrono::Utc::now(),
        },
    );

    let cost_data = CostData {
        resource_data,
        ..Default::default()
    };

    let allocations = cave_cost::compute_allocation(&cost_data).await;

    assert!(allocations.is_some());
    let alloc = allocations.unwrap();
    assert_eq!(alloc.len(), 1);
    let node_alloc = &alloc["node1"];

    // Expected cost: (2.0 * 0.05) + (4.0 * 0.01) = 0.10 + 0.04 = 0.14
    assert!((node_alloc.total_cost - 0.14).abs() < 1e-6);
    assert!((node_alloc.cpu_cost - 0.10).abs() < 1e-6);
    assert!((node_alloc.ram_cost - 0.04).abs() < 1e-6);
    assert_eq!(node_alloc.hours, node_hours);
}
```

## Implementation skeleton

```rust
pub async fn compute_allocation(cost_data: &CostData) -> Option<HashMap<String, ComputeAllocation>> {
    todo!("Tier 2")
}
```
