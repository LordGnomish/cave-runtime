---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/protocol/scim/SCIMProviderFactory.java
upstream_fn: parseScimUser
status: draft
tier: 1
created_at: 2026-04-24T16:27:33.505128+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/protocol/scim/SCIMProviderFactory.java` → `parseScimUser`

## Failing test

```rust
#[tokio::test]
async fn test_parsescimuser() {
    use cave_auth::scim::ScimUser;
    use serde_json::json;

    let json_input = json!({
        "id": "user123",
        "userName": "alice",
        "name": {
            "givenName": "Alice",
            "familyName": "Smith"
        },
        "emails": [
            {"value": "alice@example.com", "primary": true}
        ],
        "active": true,
        "password": "secret123"
    });

    let user = parsescimuser(&json_input).expect("Failed to parse SCIM user");

    assert_eq!(user.id, Some("user123".to_string()));
    assert_eq!(user.user_name, "alice");
    assert_eq!(user.given_name, Some("Alice".to_string()));
    assert_eq!(user.family_name, Some("Smith".to_string()));
    assert_eq!(user.emails.len(), 1);
    assert_eq!(user.emails[0].value, "alice@example.com");
    assert!(user.emails[0].primary);
    assert!(user.active);
    assert_eq!(user.password, Some("secret123".to_string()));
}
```

## Implementation skeleton

```rust
pub fn parsescimuser(json_value: &serde_json::Value) -> Result<ScimUser, String> {
    todo!("Tier 2");
}
```
