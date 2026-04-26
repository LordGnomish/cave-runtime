---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/costmodel.go
upstream_fn: CPUCostRate
status: draft
tier: 1
created_at: 2026-04-24T17:13:57.056372+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/costmodel.go` → `CPUCostRate`

## Failing test

```rust
#[tokio::test]
async fn test_cpucostrate() {
    use cave_cost::cpucostrate;
    use std::time::{Duration, SystemTime};

    // Simulate a pod that ran for 1 hour with 2 CPU cores at 50% utilization
    let start = SystemTime::now() - Duration::from_secs(3600);
    let end = SystemTime::now();
    let cpu_cores = 2.0;
    let cpu_utilization = 0.5; // 50%
    let cpu_price_per_core_hour = 0.05; // $0.05 per core-hour

    let result = cpucostrate(
        start,
        end,
        cpu_cores,
        cpu_utilization,
        cpu_price_per_core_hour,
    )
    .await;

    // Expected: 2 cores * 0.5 utilization * 1 hour * $0.05/core-hour = $0.05
    assert!(result.abs() < f64::EPSILON || (result - 0.05).abs() < 1e-9);
}
```

## Implementation skeleton

```rust
pub async fn cpucostrate(
    start: SystemTime,
    end: SystemTime,
    cpu_cores: f64,
    cpu_utilization: f64,
    cpu_price_per_core_hour: f64,
) -> f64 {
    todo!("Tier 2")
}
```
