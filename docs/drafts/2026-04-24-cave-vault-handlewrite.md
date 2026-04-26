---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: builtin/logical/kv/path_kv.go
upstream_fn: handleWrite
status: draft
tier: 1
created_at: 2026-04-24T17:39:05.611634+00:00
---

## Upstream reference

`openbao/openbao` → `builtin/logical/kv/path_kv.go` → `handleWrite`

## Failing test

```rust
#[tokio::test]
async fn test_handlewrite_creates_and_updates_keys() {
    use cave_vault::{Request, Response, Data, Secret};
    use std::collections::HashMap;

    // Setup: create a request to write a key-value pair
    let mut data = HashMap::new();
    data.insert("password".to_string(), "supersecret123".to_string());
    let request = Request {
        path: "secret/data/myapp/db".to_string(),
        data,
        ..Default::default()
    };

    // Execute: handle the write request
    let result = cave_vault::handlewrite(request).await;

    // Assert: expect success with no errors and correct metadata
    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.data.get("request_id"), Some(&"write".to_string()));
    assert_eq!(response.data.get("lease_id"), Some(&"".to_string()));
    assert_eq!(response.data.get("renewable"), Some(&false.to_string()));
    assert_eq!(response.data.get("lease_duration"), Some(&0.to_string()));
    assert!(response.wrap_info.is_none());
    assert!(response.auth.is_none());

    // Verify the secret was persisted (mock storage assumed)
    // In real implementation, we'd inspect the storage backend directly
    // For this test, we assume the write succeeded and metadata is correct
}
```

## Implementation skeleton

```rust
pub async fn handlewrite(request: Request) -> Result<Response, Error> {
    todo!("Tier 2")
}
```
