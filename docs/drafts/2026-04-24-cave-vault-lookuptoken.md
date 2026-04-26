---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: vault/token_store.go
upstream_fn: LookupToken
status: draft
tier: 1
created_at: 2026-04-24T17:38:17.909603+00:00
---

## Upstream reference

`openbao/openbao` → `vault/token_store.go` → `LookupToken`

## Failing test

```rust
#[tokio::test]
async fn test_lookuptoken_success_and_errors() {
    use cave_vault::{Vault, TokenStore, TokenEntry, TokenStoreError};
    use std::time::{Duration, SystemTime};
    use uuid::Uuid;

    // Initialize vault with in-memory storage
    let vault = Vault::new_in_memory();
    let token_store = vault.token_store();

    // Create a parent token
    let parent_token = "parent-token-123";
    let parent_entry = TokenEntry {
        id: parent_token.to_string(),
        accessor: Uuid::new_v4().to_string(),
        display_name: "parent".to_string(),
        policies: vec!["default".to_string(), "admin".to_string()],
        meta: std::collections::HashMap::new(),
        orphan: true,
        renewable: true,
        ttl: Duration::from_secs(3600),
        creation_time: SystemTime::now(),
        explicit_max_ttl: Duration::from_secs(86400),
        role_name: None,
    };
    token_store.insert(parent_entry.clone()).unwrap();

    // Create a child token
    let child_token = "child-token-456";
    let child_entry = TokenEntry {
        id: child_token.to_string(),
        accessor: Uuid::new_v4().to_string(),
        display_name: "child".to_string(),
        policies: vec!["default".to_string()],
        meta: std::collections::HashMap::from([("user".to_string(), "alice".to_string())]),
        orphan: false,
        renewable: true,
        ttl: Duration::from_secs(1800),
        creation_time: SystemTime::now(),
        explicit_max_ttl: Duration::from_secs(86400),
        role_name: None,
    };
    token_store.insert(child_entry.clone()).unwrap();

    // Lookup parent token
    let result = token_store.lookup_token(parent_token).await;
    assert!(result.is_ok());
    let entry = result.unwrap();
    assert_eq!(entry.id, parent_token);
    assert_eq!(entry.policies, vec!["default", "admin"]);
    assert!(entry.orphan);
    assert_eq!(entry.meta.get("user"), None);

    // Lookup child token
    let result = token_store.lookup_token(child_token).await;
    assert!(result.is_ok());
    let entry = result.unwrap();
    assert_eq!(entry.id, child_token);
    assert_eq!(entry.policies, vec!["default"]);
    assert!(!entry.orphan);
    assert_eq!(entry.meta.get("user"), Some(&"alice".to_string()));

    // Lookup non-existent token
    let result = token_store.lookup_token("nonexistent-token").await;
    assert!(matches!(result, Err(TokenStoreError::TokenNotFound)));

    // Lookup with empty string
    let result = token_store.lookup_token("").await;
    assert!(matches!(result, Err(TokenStoreError::InvalidToken)));
}
```

## Implementation skeleton

```rust
use crate::{TokenEntry, TokenStoreError};
use std::time::{Duration, SystemTime};

impl TokenStore {
    pub async fn lookup_token(&self, token: &str) -> Result<TokenEntry, TokenStoreError> {
        todo!("Tier 2")
    }
}
```
