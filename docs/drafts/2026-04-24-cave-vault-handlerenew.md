---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: vault/logical_system_paths.go
upstream_fn: handleRenew
status: draft
tier: 1
created_at: 2026-04-24T18:20:48.859741+00:00
---

## Upstream reference

`openbao/openbao` → `vault/logical_system_paths.go` → `handleRenew`

## Failing test

```rust
#[tokio::test]
async fn test_handlerenew() {
    use cave_vault::logical::{Request, Response};
    use cave_vault::secrets::kv::KvV2;
    use cave_vault::state::State;
    use cave_vault::vault::Vault;
    use std::collections::HashMap;
    use uuid::Uuid;

    // Setup vault with in-memory backend
    let vault = Vault::new();
    let state = State::new(vault);

    // Create a test secret in KV v2
    let path = "secret/data/test-renew".to_string();
    let secret_data = serde_json::json!({
        "data": {
            "foo": "bar",
            "ttl": 300
        },
        "metadata": {
            "created_time": "2024-01-01T00:00:00Z",
            "deletion_time": "",
            "version": 1
        }
    });

    // Manually insert into KV v2 backend (simulating prior write)
    let mut kv_backend = KvV2::new();
    let secret_id = Uuid::new_v4().to_string();
    let lease_id = format!("secret/data/test-renew/{}", secret_id);
    
    kv_backend.write(
        "secret".to_string(),
        "data".to_string(),
        "test-renew".to_string(),
        secret_data.clone(),
        300,
        false,
    ).await.unwrap();

    // Prepare renewal request
    let mut req = Request::new();
    req.path = "renew".to_string();
    req.data = Some(serde_json::json!({
        "lease_id": lease_id,
        "increment": 60
    }));

    // Call handlerenew
    let resp = cave_vault::logical::system_paths::handlerenew(&state, &req).await.unwrap();

    // Assert response contains renewed lease_id and lease_duration
    assert!(resp.lease_id.is_some());
    assert_eq!(resp.lease_id, Some(lease_id.clone()));
    assert!(resp.data.is_some());
    let data = resp.data.unwrap();
    assert_eq!(data["lease_id"], serde_json::Value::String(lease_id));
    assert!(data["lease_duration"].as_i64().unwrap() >= 60);
}
```

## Implementation skeleton

```rust
pub async fn handlerenew(state: &State, req: &Request) -> Result<Response, Error> {
    todo!("Tier 2")
}
```
