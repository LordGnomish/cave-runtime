---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/allocation.go
upstream_fn: IsIdle
status: draft
tier: 1
created_at: 2026-04-24T17:16:08.875986+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/allocation.go` → `IsIdle`

## Failing test

```rust
#[tokio::test]
async fn test_isidle() {
    use cave_cost::isidle;
    use cave_core::time::Duration;
    use std::time::SystemTime;

    // Create a realistic allocation window: 1 hour
    let start = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
    let end = start + Duration::from_secs(3600).as_duration();

    // Case 1: No activity at all → idle
    let empty_points = vec![];
    assert!(isidle(&empty_points, start, end, 0.01).await);

    // Case 2: Single point with zero usage → idle
    let zero_points = vec![(start + Duration::from_secs(1800).as_duration(), 0.0)];
    assert!(isidle(&zero_points, start, end, 0.01).await);

    // Case 3: Activity below threshold → idle
    let low_points = vec![(start + Duration::from_secs(100).as_duration(), 0.005)];
    assert!(isidle(&low_points, start, end, 0.01).await);

    // Case 4: Activity above threshold → not idle
    let high_points = vec![(start + Duration::from_secs(200).as_duration(), 0.02)];
    assert!(!isidle(&high_points, start, end, 0.01).await);

    // Case 5: Mixed activity, but max below threshold → idle
    let mixed_points = vec![
        (start + Duration::from_secs(100).as_duration(), 0.003),
        (start + Duration::from_secs(200).as_duration(), 0.007),
        (start + Duration::from_secs(300).as_duration(), 0.009),
    ];
    assert!(isidle(&mixed_points, start, end, 0.01).await);

    // Case 6: Mixed activity, max exceeds threshold → not idle
    let mixed_high_points = vec![
        (start + Duration::from_secs(100).as_duration(), 0.003),
        (start + Duration::from_secs(200).as_duration(), 0.015),
        (start + Duration::from_secs(300).as_duration(), 0.009),
    ];
    assert!(!isidle(&mixed_high_points, start, end, 0.01).await);
}
```

## Implementation skeleton

```rust
pub async fn isidle(
    points: &[(u64, f64)],
    window_start: u64,
    window_end: u64,
    threshold: f64,
) -> bool {
    todo!("Tier 2")
}
```
