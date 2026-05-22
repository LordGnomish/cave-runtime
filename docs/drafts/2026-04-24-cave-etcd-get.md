---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/storage/mvcc/index.go
upstream_fn: Get
status: draft
tier: 1
created_at: 2026-04-24T17:18:15.295935+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/storage/mvcc/index.go` → `Get`

## Failing test

```rust
#[tokio::test]
async fn test_get_existing_key() {
    use cave_etcd::store::Store;
    use cave_etcd::kv::KeyValue;
    use cave_etcd::mvcc::index::Index;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // Setup: create a store with a single key-value pair
    let store = Arc::new(RwLock::new(Store::default()));
    let index = Arc::new(RwLock::new(Index::default()));

    // Put a key with revision 1
    let mut store_guard = store.write().await;
    store_guard.put(b"foo".to_vec(), b"bar".to_vec(), 1, None);
    drop(store_guard);

    // Index the key
    let mut index_guard = index.write().await;
    index_guard.insert(b"foo".to_vec(), 1, 1);
    drop(index_guard);

    // Call get for key "foo" at revision 1
    let result = cave_etcd::get::get(
        store.clone(),
        index.clone(),
        b"foo".to_vec(),
        1,
        false,
    )
    .await;

    assert!(result.is_ok());
    let (kv, rev) = result.unwrap();
    assert_eq!(rev, 1);
    assert_eq!(kv.key, b"foo".to_vec());
    assert_eq!(kv.value, Some(b"bar".to_vec()));
    assert_eq!(kv.create_revision, 1);
    assert_eq!(kv.mod_revision, 1);

    // Test get non-existing key
    let result = cave_etcd::get::get(
        store.clone(),
        index.clone(),
        b"baz".to_vec(),
        1,
        false,
    )
    .await;

    assert!(result.is_ok());
    let (kv, _) = result.unwrap();
    assert!(kv.key.is_empty());
    assert!(kv.value.is_none());
}
```

## Implementation skeleton

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::store::Store;
use crate::mvcc::index::Index;
use crate::kv::KeyValue;

pub async fn get(
    store: Arc<RwLock<Store>>,
    index: Arc<RwLock<Index>>,
    key: Vec<u8>,
    rev: i64,
    count_only: bool,
) -> Result<(KeyValue, i64), String> {
    todo!("Tier 2");
}
```
