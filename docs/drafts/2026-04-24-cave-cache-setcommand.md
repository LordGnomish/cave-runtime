---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/t_string.c
upstream_fn: setCommand
status: draft
tier: 1
created_at: 2026-04-24T16:44:25.969707+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/t_string.c` → `setCommand`

## Failing test

```rust
#[tokio::test]
async fn test_setcommand_basic() {
    use cave_cache::{Cache, Value, Error};
    use std::time::Duration;

    let cache = Cache::new();
    
    // Basic SET
    let result = cache.setcommand("key1".into(), "value1".into(), None, None, false).await;
    assert!(matches!(result, Ok(Value::Ok)));

    // Verify value was set
    let get_result = cache.get("key1".into()).await;
    assert_eq!(get_result, Ok(Value::BulkString("value1".into())));

    // SET with EX
    let result = cache.setcommand("key2".into(), "value2".into(), Some(Duration::from_secs(10)), None, false).await;
    assert!(matches!(result, Ok(Value::Ok)));

    // SET with PX
    let result = cache.setcommand("key3".into(), "value3".into(), None, Some(Duration::from_millis(5000)), false).await;
    assert!(matches!(result, Ok(Value::Ok)));

    // SET with NX (should set since key doesn't exist)
    let result = cache.setcommand("key4".into(), "value4".into(), None, None, true).await;
    assert!(matches!(result, Ok(Value::Ok)));

    // SET with NX (should not set since key exists)
    let result = cache.setcommand("key1".into(), "new_value1".into(), None, None, true).await;
    assert!(matches!(result, Ok(Value::Nil)));

    // SET with XX (should not set since key5 doesn't exist)
    let result = cache.setcommand("key5".into(), "value5".into(), None, None, false).await;
    assert!(matches!(result, Ok(Value::Nil)));

    // SET with XX (should set since key1 exists)
    let result = cache.setcommand("key1".into(), "updated_value1".into(), None, None, false).await;
    assert!(matches!(result, Ok(Value::Ok)));

    // Verify final value
    let get_result = cache.get("key1".into()).await;
    assert_eq!(get_result, Ok(Value::BulkString("updated_value1".into())));
}
```

## Implementation skeleton

```rust
pub async fn setcommand(
    &self,
    key: Value,
    value: Value,
    ex: Option<Duration>,
    px: Option<Duration>,
    nx: bool,
) -> Result<Value, Error> {
    todo!("Tier 2")
}
```
