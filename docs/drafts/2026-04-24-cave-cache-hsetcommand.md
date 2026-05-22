---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/t_hash.c
upstream_fn: hsetCommand
status: draft
tier: 1
created_at: 2026-04-24T16:45:25.547023+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/t_hash.c` → `hsetCommand`

## Failing test

```rust
#[tokio::test]
async fn test_hsetcommand() {
    use cave_cache::{Command, Database, Value};
    use std::collections::HashMap;

    let mut db = Database::new();
    let key = "myhash";
    let field1 = "field1";
    let value1 = "value1";
    let field2 = "field2";
    let value2 = "value2";
    let new_value = "newvalue";

    // Initial hset: create hash with two fields
    let cmd = Command::HSet {
        key: key.to_string(),
        field_value_pairs: vec![(field1.to_string(), value1.to_string()), (field2.to_string(), value2.to_string())],
    };
    let result = db.execute(cmd).await.unwrap();
    assert_eq!(result, Value::Integer(2));

    // Verify hash contents
    let cmd = Command::HGetAll { key: key.to_string() };
    let result = db.execute(cmd).await.unwrap();
    let expected_map: HashMap<String, String> = [
        (field1.to_string(), value1.to_string()),
        (field2.to_string(), value2.to_string()),
    ]
    .into_iter()
    .collect();
    match result {
        Value::Map(map) => {
            assert_eq!(map.len(), 2);
            assert_eq!(map.get(field1), Some(&Value::BulkString(value1.to_string())));
            assert_eq!(map.get(field2), Some(&Value::BulkString(value2.to_string())));
        }
        _ => panic!("Expected Map, got {:?}", result),
    }

    // Update one field and add another
    let cmd = Command::HSet {
        key: key.to_string(),
        field_value_pairs: vec![(field1.to_string(), new_value.to_string()), ("field3".to_string(), "value3".to_string())],
    };
    let result = db.execute(cmd).await.unwrap();
    assert_eq!(result, Value::Integer(1)); // 1 new field added, 1 updated

    // Verify updated contents
    let cmd = Command::HGetAll { key: key.to_string() };
    let result = db.execute(cmd).await.unwrap();
    match result {
        Value::Map(map) => {
            assert_eq!(map.len(), 3);
            assert_eq!(map.get(field1), Some(&Value::BulkString(new_value.to_string())));
            assert_eq!(map.get("field3"), Some(&Value::BulkString("value3".to_string())));
        }
        _ => panic!("Expected Map, got {:?}", result),
    }

    // Test with single field-value pair
    let cmd = Command::HSet {
        key: "single".to_string(),
        field_value_pairs: vec![("f".to_string(), "v".to_string())],
    };
    let result = db.execute(cmd).await.unwrap();
    assert_eq!(result, Value::Integer(1));
}
```

## Implementation skeleton

```rust
pub async fn hsetcommand(
    db: &mut Database,
    key: String,
    field_value_pairs: Vec<(String, String)>,
) -> Result<Value, Error> {
    todo!("Tier 2")
}
```
