---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/t_list.c
upstream_fn: lrangeCommand
status: draft
tier: 1
created_at: 2026-04-24T16:46:25.528526+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/t_list.c` → `lrangeCommand`

## Failing test

```rust
#[tokio::test]
async fn test_lrangecommand() {
    use cave_cache::{Cache, Error};
    use cave_cache::command::Command;
    use cave_cache::value::Value;

    let cache = Cache::default();
    
    // Populate list with 5 elements
    cache.execute(Command::Push {
        key: "mylist".into(),
        value: Value::BulkString(b"one".to_vec()),
        tail: false,
    }).await.unwrap();
    cache.execute(Command::Push {
        key: "mylist".into(),
        value: Value::BulkString(b"two".to_vec()),
        tail: false,
    }).await.unwrap();
    cache.execute(Command::Push {
        key: "mylist".into(),
        value: Value::BulkString(b"three".to_vec()),
        tail: false,
    }).await.unwrap();
    cache.execute(Command::Push {
        key: "mylist".into(),
        value: Value::BulkString(b"four".to_vec()),
        tail: false,
    }).await.unwrap();
    cache.execute(Command::Push {
        key: "mylist".into(),
        value: Value::BulkString(b"five".to_vec()),
        tail: false,
    }).await.unwrap();

    // Test basic range: lrange mylist 0 2
    let result = cache.execute(Command::Range {
        key: "mylist".into(),
        start: 0,
        stop: 2,
    }).await;
    assert_eq!(result, Ok(Value::Array(vec![
        Value::BulkString(b"one".to_vec()),
        Value::BulkString(b"two".to_vec()),
        Value::BulkString(b"three".to_vec()),
    ])));

    // Test negative indices: lrange mylist -3 -1
    let result = cache.execute(Command::Range {
        key: "mylist".into(),
        start: -3,
        stop: -1,
    }).await;
    assert_eq!(result, Ok(Value::Array(vec![
        Value::BulkString(b"three".to_vec()),
        Value::BulkString(b"four".to_vec()),
        Value::BulkString(b"five".to_vec()),
    ])));

    // Test out-of-range: lrange mylist 0 100
    let result = cache.execute(Command::Range {
        key: "mylist".into(),
        start: 0,
        stop: 100,
    }).await;
    assert_eq!(result, Ok(Value::Array(vec![
        Value::BulkString(b"one".to_vec()),
        Value::BulkString(b"two".to_vec()),
        Value::BulkString(b"three".to_vec()),
        Value::BulkString(b"four".to_vec()),
        Value::BulkString(b"five".to_vec()),
    ])));

    // Test empty range: lrange mylist 5 3
    let result = cache.execute(Command::Range {
        key: "mylist".into(),
        start: 5,
        stop: 3,
    }).await;
    assert_eq!(result, Ok(Value::Array(vec![])));

    // Test non-existent key
    let result = cache.execute(Command::Range {
        key: "nonexistent".into(),
        start: 0,
        stop: -1,
    }).await;
    assert_eq!(result, Ok(Value::Array(vec![])));
}
```

## Implementation skeleton

```rust
pub async fn lrangecommand(cache: &Cache, key: &[u8], start: i64, stop: i64) -> Result<Value, Error> {
    todo!("Tier 2")
}
```
