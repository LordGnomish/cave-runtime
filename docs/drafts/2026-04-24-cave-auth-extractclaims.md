---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: core/src/main/java/org/keycloak/representations/idm/ClaimRepresentation.java
upstream_fn: extractClaims
status: draft
tier: 1
created_at: 2026-04-24T16:26:10.214666+00:00
---

## Upstream reference

`keycloak/keycloak` → `core/src/main/java/org/keycloak/representations/idm/ClaimRepresentation.java` → `extractClaims`

## Failing test

```rust
#[tokio::test]
async fn test_extractclaims_extracts_claims_from_token() {
    use cave_auth::extractclaims;
    use serde_json::json;

    // Simulate a JWT-like token payload with claims
    let token_payload = json!({
        "sub": "user123",
        "email": "user@example.com",
        "realm_access": {
            "roles": ["user", "admin"]
        },
        "resource_access": {
            "account": {
                "roles": ["manage-account"]
            }
        },
        "preferred_username": "user123"
    });

    let claims = extractclaims(token_payload).await;

    assert_eq!(claims.get("sub"), Some(&json!("user123")));
    assert_eq!(claims.get("email"), Some(&json!("user@example.com")));
    assert_eq!(claims.get("preferred_username"), Some(&json!("user123")));
    assert_eq!(
        claims.get("realm_access"),
        Some(&json!({"roles": ["user", "admin"]}))
    );
    assert_eq!(
        claims.get("resource_access"),
        Some(&json!({"account": {"roles": ["manage-account"]}}))
    );
}
```

## Implementation skeleton

```rust
use serde_json::Value;

pub async fn extractclaims(token_payload: Value) -> std::collections::HashMap<String, Value> {
    todo!("Tier 2")
}
```
