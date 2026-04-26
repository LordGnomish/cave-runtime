---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/window.go
upstream_fn: ParseWindowUnsafe
status: draft
tier: 1
created_at: 2026-04-24T17:14:53.824298+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/window.go` → `ParseWindowUnsafe`

## Failing test

```rust
#[tokio::test]
async fn test_parsewindowunsafe() {
    use cave_cost::parsewindowunsafe;
    use std::time::{Duration, SystemTime};

    // Test case 1: Valid window with start and end
    let window_str = "2023-01-01T00:00:00Z/2023-01-01T01:00:00Z";
    let (start, end) = parsewindowunsafe(window_str).await.unwrap();
    let expected_start = SystemTime::UNIX_EPOCH + Duration::from_secs(1672531200); // 2023-01-01T00:00:00Z
    let expected_end = SystemTime::UNIX_EPOCH + Duration::from_secs(1672534800);   // 2023-01-01T01:00:00Z
    assert_eq!(start, expected_start);
    assert_eq!(end, expected_end);

    // Test case 2: Valid window with duration (P1H)
    let window_str = "2023-01-01T00:00:00Z/P1H";
    let (start, end) = parsewindowunsafe(window_str).await.unwrap();
    assert_eq!(start, expected_start);
    assert_eq!(end, expected_end);

    // Test case 3: Invalid input (malformed)
    let window_str = "invalid-window";
    let result = parsewindowunsafe(window_str).await;
    assert!(result.is_err());

    // Test case 4: Empty string
    let window_str = "";
    let result = parsewindowunsafe(window_str).await;
    assert!(result.is_err());
}
```

## Implementation skeleton

```rust
use std::time::{Duration, SystemTime};
use anyhow::Result;

pub async fn parsewindowunsafe(window: &str) -> Result<(SystemTime, SystemTime)> {
    todo!("Tier 2")
}
```
