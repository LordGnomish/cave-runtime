---
crate: cave-cache
upstream_repo: valkey-io/valkey
upstream_file: src/evict.c
upstream_fn: evict
status: draft
tier: 1
created_at: 2026-04-24T18:19:23.340156+00:00
---

## Upstream reference

`valkey-io/valkey` → `src/evict.c` → `evict`

## Failing test

```rust
#[tokio::test]
async fn evict_removes_least_recently_used_entries_when_memory_exceeded() {
    use cave_cache::{Cache, EvictionPolicy};
    use std::time::Duration;

    // Create a cache with a small max memory (1KB) and LRU eviction
    let mut cache = Cache::new(
        1024, // max_memory_bytes
        EvictionPolicy::Lru,
    );

    // Insert entries that exceed the memory limit
    cache.insert("key1", vec![b'a'; 300]).await;
    cache.insert("key2", vec![b'b'; 300]).await;
    cache.insert("key3", vec![b'c'; 300]).await;
    cache.insert("key4", vec![b'd'; 300]).await; // This should trigger eviction

    // Access key1 to make it recently used
    let _ = cache.get("key1").await;

    // Insert key5 to push memory usage over limit again
    cache.insert("key5", vec![b'e'; 300]).await;

    // key2 should be evicted (least recently used after key1 was accessed)
    assert!(cache.contains_key("key1").await);
    assert!(!cache.contains_key("key2").await);
    assert!(cache.contains_key("key3").await);
    assert!(cache.contains_key("key4").await);
    assert!(cache.contains_key("key5").await);

    // Verify memory usage is within bounds
    assert!(cache.used_memory().await <= 1024);
}
```

## Implementation skeleton

```rust
pub async fn evict(cache: &mut Cache, needed_bytes: usize) -> usize {
    todo!("Tier 2")
}
```
