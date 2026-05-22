---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/storage/mvcc/kvstore.go
upstream_fn: parseKey
status: draft
tier: 1
created_at: 2026-04-24T16:37:16.565060+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/storage/mvcc/kvstore.go` → `parseKey`

## Failing test

```rust
#[tokio::test]
async fn test_parsekey() {
    use cave_etcd::parsekey;
    
    // Test valid key with revision
    let (key, rev) = parsekey(b"key1\x00\x00\x00\x00\x00\x00\x00\x01").unwrap();
    assert_eq!(key, b"key1");
    assert_eq!(rev, 1);

    // Test valid key with larger revision
    let (key, rev) = parsekey(b"another_key\x00\x00\x00\x00\x00\x00\x00\x0a").unwrap();
    assert_eq!(key, b"another_key");
    assert_eq!(rev, 10);

    // Test empty key (invalid)
    let result = parsekey(b"\x00\x00\x00\x00\x00\x00\x00\x01");
    assert!(result.is_err());

    // Test key without revision (too short)
    let result = parsekey(b"short");
    assert!(result.is_err());

    // Test key with zero revision (invalid per etcd semantics)
    let result = parsekey(b"key2\x00\x00\x00\x00\x00\x00\x00\x00");
    assert!(result.is_err());
}
```

## Implementation skeleton

```rust
pub fn parsekey(data: &[u8]) -> Result<(Vec<u8>, u64), &'static str> {
    if data.len() < 8 {
        return Err("data too short to contain revision");
    }
    
    let (key_part, rev_bytes) = data.split_at(data.len() - 8);
    
    if key_part.is_empty() {
        return Err("key cannot be empty");
    }
    
    let rev = u64::from_be_bytes(rev_bytes.try_into().map_err(|_| "invalid revision bytes")?);
    
    if rev == 0 {
        return Err("revision must be non-zero");
    }
    
    Ok((key_part.to_vec(), rev))
}
```
