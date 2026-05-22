---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/protocol/oidc/TokenManager.java
upstream_fn: buildAccessToken
status: draft
tier: 1
created_at: 2026-04-24T17:17:00.410925+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/protocol/oidc/TokenManager.java` → `buildAccessToken`

## Failing test

```rust
#[tokio::test]
async fn test_buildaccesstoken() {
    use cave_auth::{AccessToken, AccessTokenBuilder, Client, ClientId, Realm, UserId, Timestamp};
    use std::time::{Duration, SystemTime};

    // Setup realistic inputs
    let realm = Realm::new("master".to_string());
    let client = Client::new(ClientId::new("account".to_string()));
    let user_id = UserId::new("a1b2c3d4-e5f6-7890-1234-567890abcdef".to_string());
    let issued_at = SystemTime::now();
    let expires_in = Duration::from_secs(3600);
    let scope = "openid profile email".to_string();
    let nonce = Some("n-0S6_WzA2Mj".to_string());
    let amr = vec!["password".to_string(), "mfa".to_string()];

    // Build access token
    let token = buildaccesstoken(
        &realm,
        &client,
        &user_id,
        issued_at,
        expires_in,
        &scope,
        nonce.as_deref(),
        &amr,
    ).await;

    // Assert expected structure
    assert!(token.issued_at() >= issued_at);
    assert!(token.expires_at() >= token.issued_at());
    assert_eq!(token.issued_at().duration_since(issued_at).unwrap(), Duration::ZERO);
    assert_eq!(token.expires_at().duration_since(token.issued_at()).unwrap(), expires_in);
    assert_eq!(token.scope(), Some(&scope));
    assert_eq!(token.nonce(), nonce.as_deref());
    assert!(token.amr().contains(&"password".to_string()));
    assert!(token.amr().contains(&"mfa".to_string()));
    assert_eq!(token.realm(), &realm);
    assert_eq!(token.client_id(), &client.id());
    assert_eq!(token.subject(), &user_id);
}
```

## Implementation skeleton

```rust
pub async fn buildaccesstoken(
    realm: &Realm,
    client: &Client,
    user_id: &UserId,
    issued_at: SystemTime,
    expires_in: Duration,
    scope: &str,
    nonce: Option<&str>,
    amr: &[String],
) -> AccessToken {
    todo!("Tier 2")
}
```
