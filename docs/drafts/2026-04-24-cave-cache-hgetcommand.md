---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/t_hash.c
upstream_fn: hgetCommand
status: draft
tier: 1
created_at: 2026-04-24T16:44:52.786787+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/t_hash.c` → `hgetCommand`

## Failing test

```rust
#[tokio::test]
async fn test_hgetcommand() {
    use cave_cache::{Command, Context, Reply, Value};
    use std::collections::HashMap;

    // Setup: create a mock context with a hash key "myhash" containing field-value pairs
    let mut ctx = Context::default();
    let mut hash_map = HashMap::new();
    hash_map.insert("field1".to_string(), Value::BulkString(b"value1".to_vec()));
    hash_map.insert("field2".to_string(), Value::BulkString(b"value2".to_vec()));
    ctx.db.insert("myhash".to_string(), Value::Hash(hash_map));

    // Test 1: get existing field
    let cmd = Command::new("HGET".to_string(), vec![
        Value::BulkString(b"myhash".to_vec()),
        Value::BulkString(b"field1".to_vec()),
    ]);
    let result = hgetcommand(&mut ctx, &cmd).await;
    assert_eq!(result, Reply::BulkString(Some(b"value1".to_vec())));

    // Test 2: get non-existing field (should return nil)
    let cmd = Command::new("HGET".to_string(), vec![
        Value::BulkString(b"myhash".to_vec()),
        Value::BulkString(b"nonexistent".to_vec()),
    ]);
    let result = hgetcommand(&mut ctx, &cmd).await;
    assert_eq!(result, Reply::Null);

    // Test 3: get from non-existing key (should return nil)
    let cmd = Command::new("HGET".to_string(), vec![
        Value::BulkString(b"nonexistent_key".to_vec()),
        Value::BulkString(b"field1".to_vec()),
    ]);
    let result = hgetcommand(&mut ctx, &cmd).await;
    assert_eq!(result, Reply::Null);

    // Test 4: wrong number of arguments (should return error)
    let cmd = Command::new("HGET".to_string(), vec![
        Value::BulkString(b"myhash".to_vec()),
    ]);
    let result = hgetcommand(&mut ctx, &cmd).await;
    assert!(matches!(result, Reply::Error(_)));
}
```

## Implementation skeleton

```rust
pub async fn hgetcommand(ctx: &mut Context, cmd: &Command) -> Reply {
    todo!("Tier 2")
}
```
