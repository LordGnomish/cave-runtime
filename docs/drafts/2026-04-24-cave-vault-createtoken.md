---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: vault/token_store.go
upstream_fn: CreateToken
status: draft
tier: 1
created_at: 2026-04-24T17:38:45.541287+00:00
---

## Upstream reference

`openbao/openbao` → `vault/token_store.go` → `CreateToken`

## Failing test

```rust
#[tokio::test]
async fn test_createtoken_creates_valid_token_with_expected_properties() {
    use cave_vault::{createtoken, TokenStore, TokenCreateRequest};
    use cave_core::{error::Error, identity::Identity, time::SystemTime};
    use std::time::Duration;

    // Setup: create a token store with a root token
    let store = TokenStore::new();
    let root_token = store.create_root_token().await.unwrap();

    // Prepare request to create a child token
    let request = TokenCreateRequest {
        accessor: None,
        id: None,
        policies: vec!["default".to_string(), "dev".to_string()],
        metadata: Some(vec![("user".to_string(), "alice".to_string())].into_iter().collect()),
        lease: Some(Duration::from_secs(3600)),
        explicit_max_ttl: Some(Duration::from_secs(7200)),
        renewable: Some(true),
        type_: Some("service".to_string()),
        orphan: Some(true),
        no_default_policy: Some(false),
        period: None,
        entity_alias: None,
    };

    // Act: create token using root identity
    let result = createtoken(&store, &root_token, &request).await;

    // Assert
    assert!(result.is_ok(), "token creation should succeed");
    let token_info = result.unwrap();
    assert_eq!(token_info.policies.len(), 2);
    assert!(token_info.policies.contains(&"default".to_string()));
    assert!(token_info.policies.contains(&"dev".to_string()));
    assert_eq!(token_info.metadata.get("user"), Some(&"alice".to_string()));
    assert!(token_info.renewable);
    assert_eq!(token_info.type_, "service");
    assert!(token_info.orphan);
    assert_eq!(token_info.lease_duration, Some(3600));
    assert_eq!(token_info.explicit_max_ttl, Some(7200));
    assert!(token_info.id.len() > 0);
    assert!(token_info.accessor.len() > 0);
    assert_ne!(token_info.id, token_info.accessor);
    assert!(token_info.creation_time > SystemTime::UNIX_EPOCH);
}
```

## Implementation skeleton

```rust
pub async fn createtoken(
    store: &TokenStore,
    parent_token: &str,
    request: &TokenCreateRequest,
) -> Result<TokenInfo, Error> {
    todo!("Tier 2")
}
```
