---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/window.go
upstream_fn: Duration
status: draft
tier: 1
created_at: 2026-04-24T17:15:12.028984+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/window.go` → `Duration`

## Failing test

```rust
#[tokio::test]
async fn test_duration() {
    use cave_cost::duration;
    use std::time::Duration;

    // Test case 1: basic duration parsing from string
    let result = duration("1h30m").await;
    assert_eq!(result, Duration::from_secs(5400)); // 1.5 hours = 5400 seconds

    // Test case 2: zero duration
    let result = duration("0s").await;
    assert_eq!(result, Duration::from_secs(0));

    // Test case 3: complex duration with days, hours, minutes, seconds
    let result = duration("1d2h3m4s").await;
    assert_eq!(result, Duration::from_secs(93784)); // 1*86400 + 2*3600 + 3*60 + 4

    // Test case 4: invalid duration returns zero duration
    let result = duration("invalid").await;
    assert_eq!(result, Duration::from_secs(0));

    // Test case 5: empty string returns zero duration
    let result = duration("").await;
    assert_eq!(result, Duration::from_secs(0));
}
```

## Implementation skeleton

```rust
pub async fn duration(s: &str) -> std::time::Duration {
    todo!("Tier 2")
}
```
