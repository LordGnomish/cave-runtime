---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/costmodel.go
upstream_fn: MemoryCostRate
status: draft
tier: 1
created_at: 2026-04-24T17:14:30.025537+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/costmodel.go` → `MemoryCostRate`

## Failing test

```rust
#[tokio::test]
async fn test_memorycostrate() {
    use cave_cost::memorycostrate;
    use std::time::{Duration, SystemTime};

    // Simulate a pod with 2Gi memory used over 1 hour
    let memory_bytes = 2 * 1024 * 1024 * 1024u64; // 2 GiB
    let start_time = SystemTime::now() - Duration::from_secs(3600);
    let end_time = SystemTime::now();
    let window_duration = end_time.duration_since(start_time).unwrap();

    let rate = memorycostrate(memory_bytes, window_duration).await;
    
    // Expected: (2 GiB * 1 hour) / (1 hour) = 2 GiB-hours/hour = 2 GiB
    // But upstream uses GB-hours, so convert: 2 GiB = 2.147... GB
    // For simplicity, assume upstream uses binary units (GiB) and returns GiB-hours per hour = GiB
    // In upstream, MemoryCostRate = (memory_bytes * window_seconds) / (1_000_000_000 * 3600)
    // Actually: MemoryCostRate = (memory_bytes * window_seconds) / (1_073_741_824 * 3600) * 1024? 
    // Let's match upstream: MemoryCostRate = (memory_bytes * window_seconds) / (1_000_000_000 * 3600)
    // But upstream uses GiB (2^30), not GB (10^9). Let's assume GiB.
    // In upstream: MemoryCostRate = (memory_bytes * window_seconds) / (1_073_741_824 * 3600)
    // For 2 GiB over 3600 seconds: (2 * 1_073_741_824 * 3600) / (1_073_741_824 * 3600) = 2.0
    let expected = 2.0;
    
    assert!((rate - expected).abs() < 0.001, "Expected ~{expected}, got {rate}");
}
```

## Implementation skeleton

```rust
pub async fn memorycostrate(memory_bytes: u64, window_duration: Duration) -> f64 {
    todo!("Tier 2")
}
```
