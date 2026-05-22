---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/t_list.c
upstream_fn: lpushCommand
status: draft
tier: 1
created_at: 2026-04-24T16:45:49.154466+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/t_list.c` → `lpushCommand`

## Failing test

```rust
#[tokio::test]
async fn test_lpushcommand() {
    use cave_cache::{Database, Value, Error};
    use std::collections::HashMap;

    let mut db = Database::default();
    let key = "mylist";
    let elements = vec!["world", "hello"];
    
    // Initial lpush: creates list with "hello", then "world" prepended
    let result = lpushcommand(&mut db, key, elements.clone()).await;
    assert!(matches!(result, Ok(Value::Integer(2))));
    
    // Verify list contents: should be ["world", "hello"]
    let list = db.get_list(key).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], Value::BulkString("world".as_bytes().to_vec()));
    assert_eq!(list[1], Value::BulkString("hello".as_bytes().to_vec()));
    
    // Additional lpush: prepend "foo"
    let result = lpushcommand(&mut db, key, vec!["foo"]).await;
    assert!(matches!(result, Ok(Value::Integer(3))));
    
    // Verify updated list: ["foo", "world", "hello"]
    let list = db.get_list(key).unwrap();
    assert_eq!(list.len(), 3);
    assert_eq!(list[0], Value::BulkString("foo".as_bytes().to_vec()));
    assert_eq!(list[1], Value::BulkString("world".as_bytes().to_vec()));
    assert_eq!(list[2], Value::BulkString("hello".as_bytes().to_vec()));
    
    // Test with empty elements list (should return 0)
    let result = lpushcommand(&mut db, "empty_list", Vec::<&str>::new()).await;
    assert!(matches!(result, Ok(Value::Integer(0))));
}
```

## Implementation skeleton

```rust
pub async fn lpushcommand<'a>(db: &mut Database, key: &str, elements: Vec<&str>) -> Result<Value, Error> {
    todo!("Tier 2")
}
```
