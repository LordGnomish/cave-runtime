---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: builtin/logical/kv/path_kv.go
upstream_fn: handleDelete
status: draft
tier: 1
created_at: 2026-04-24T17:40:06.377841+00:00
---

## Upstream reference

`openbao/openbao` → `builtin/logical/kv/path_kv.go` → `handleDelete`

## Failing test

```rust
#[tokio::test]
async fn test_handledelete_deletes_key_and_returns_no_data() {
    use cave_vault::logical::{Request, Response, PathData};
    use cave_vault::storage::memory::MemoryStorage;
    use cave_vault::vault::Vault;
    use std::sync::Arc;

    // Setup vault with in-memory storage
    let storage = Arc::new(MemoryStorage::default());
    let vault = Vault::new(storage.clone());

    // Write a key first
    let write_req = Request {
        path: "secret/data/foo".to_string(),
        operation: "create".to_string(),
        data: Some(vec![("value".to_string(), "bar".to_string())].into_iter().collect()),
        ..Default::default()
    };
    let write_resp = vault.handle_request(write_req).await.unwrap();
    assert!(write_resp.is_ok());

    // Verify key exists
    let read_req = Request {
        path: "secret/data/foo".to_string(),
        operation: "read".to_string(),
        ..Default::default()
    };
    let read_resp = vault.handle_request(read_req).await.unwrap();
    assert!(read_resp.is_ok());
    assert_eq!(read_resp.data().unwrap().get("value"), Some(&"bar".to_string()));

    // Delete the key
    let delete_req = Request {
        path: "secret/data/foo".to_string(),
        operation: "delete".to_string(),
        ..Default::default()
    };
    let delete_resp = vault.handle_request(delete_req).await.unwrap();

    // Assert deletion succeeded and returned no data
    assert!(delete_resp.is_ok());
    assert!(delete_resp.data().is_none());

    // Verify key is gone
    let read_after_req = Request {
        path: "secret/data/foo".to_string(),
        operation: "read".to_string(),
        ..Default::default()
    };
    let read_after_resp = vault.handle_request(read_after_req).await.unwrap();
    assert!(read_after_resp.is_ok());
    assert!(read_after_resp.data().is_none());
}
```

## Implementation skeleton

```rust
pub async fn handledelete(
    _request: &Request,
    _vault: &Vault,
) -> Result<Response, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
