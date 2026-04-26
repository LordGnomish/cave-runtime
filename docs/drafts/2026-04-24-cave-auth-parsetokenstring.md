---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/services/DefaultKeycloakSession.java
upstream_fn: parseTokenString
status: draft
tier: 1
created_at: 2026-04-24T16:25:32.223704+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/services/DefaultKeycloakSession.java` → `parseTokenString`

## Failing test

```rust
#[tokio::test]
async fn test_parsetokenstring() {
    use cave_auth::parsetokenstring;
    use cave_core::error::Error;
    use serde_json::json;

    // Valid token with HS256 algorithm
    let valid_token = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
    
    match parsetokenstring(valid_token).await {
        Ok(token_data) => {
            assert_eq!(token_data.header.get("alg").unwrap(), "HS256");
            assert_eq!(token_data.payload.get("sub").unwrap(), "1234567890");
            assert_eq!(token_data.payload.get("name").unwrap(), "John Doe");
        }
        Err(e) => panic!("Expected success, got error: {:?}", e),
    }

    // Invalid token (malformed base64)
    let invalid_token = "not.a.valid.token!!!";
    match parsetokenstring(invalid_token).await {
        Err(Error::InvalidToken(_)) => {}
        _ => panic!("Expected InvalidToken error for malformed token"),
    }

    // Token with unsupported algorithm
    let unsupported_token = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVP-mB92k27uhbUJU1p1r_wW1gFWFOEjXo";
    match parsetokenstring(unsupported_token).await {
        Err(Error::UnsupportedAlgorithm(_)) => {}
        _ => panic!("Expected UnsupportedAlgorithm error for RS256 token"),
    }
}
```

## Implementation skeleton

```rust
pub async fn parsetokenstring(token: &str) -> Result<TokenData, Error> {
    todo!("Tier 2")
}
```
