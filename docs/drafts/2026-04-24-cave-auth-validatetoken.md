---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/services/managers/AuthenticationManager.java
upstream_fn: validateToken
status: draft
tier: 1
created_at: 2026-04-24T16:26:45.267161+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/services/managers/AuthenticationManager.java` → `validateToken`

## Failing test

```rust
#[tokio::test]
async fn test_validatetoken_valid_token() {
    use cave_auth::{validatetoken, TokenValidationResult};
    use cave_core::user::UserId;
    use std::time::{SystemTime, Duration};

    // Simulate a valid token with expiration in the future
    let now = SystemTime::now();
    let exp = now + Duration::from_secs(3600);
    let iat = now - Duration::from_secs(60);
    
    let token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyMTIzIiwiaWF0IjoxNzA0MDY3MjAwLCJleHAiOjE3MDQwNzA4MDB9.dGVzdF9zaWduYXR1cmU";
    let client_id = "cave-client";
    let realm = "cave-realm";

    let result = validatetoken(token, client_id, realm).await;

    match result {
        TokenValidationResult::Valid(user_id) => {
            assert_eq!(user_id, UserId::new("user123".to_string()));
        }
        _ => panic!("Expected valid token, got {:?}", result),
    }
}
```

## Implementation skeleton

```rust
pub async fn validatetoken(
    token: &str,
    client_id: &str,
    realm: &str,
) -> TokenValidationResult {
    todo!("Tier 2")
}
```
