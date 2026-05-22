---
crate: cave-auth
upstream_repo: keycloak/keycloak
upstream_file: services/src/main/java/org/keycloak/protocol/oidc/OIDCLoginProtocol.java
upstream_fn: buildAuthorizationUrl
status: draft
tier: 1
created_at: 2026-04-24T16:27:11.966337+00:00
---

## Upstream reference

`keycloak/keycloak` → `services/src/main/java/org/keycloak/protocol/oidc/OIDCLoginProtocol.java` → `buildAuthorizationUrl`

## Failing test

```rust
#[tokio::test]
async fn test_buildauthorizationurl() {
    use cave_auth::buildauthorizationurl;
    use url::Url;

    let result = buildauthorizationurl(
        "https://auth.example.com",
        "client-id",
        "code",
        "https://client.example.com/callback",
        "state123",
        Some("nonce456"),
        Some("login"),
        Some(vec!["email", "profile"]),
        Some(3600),
    )
    .await
    .unwrap();

    let url = Url::parse(&result).unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("auth.example.com"));
    assert_eq!(url.path(), "/realms/master/protocol/openid-connect/auth");

    let query_pairs: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query_pairs.get("client_id"), Some(&"client-id".to_string()));
    assert_eq!(query_pairs.get("response_type"), Some(&"code".to_string()));
    assert_eq!(query_pairs.get("redirect_uri"), Some(&"https://client.example.com/callback".to_string()));
    assert_eq!(query_pairs.get("state"), Some(&"state123".to_string()));
    assert_eq!(query_pairs.get("nonce"), Some(&"nonce456".to_string()));
    assert_eq!(query_pairs.get("prompt"), Some(&"login".to_string()));
    assert_eq!(query_pairs.get("scope"), Some(&"email profile".to_string()));
    assert_eq!(query_pairs.get("max_age"), Some(&"3600".to_string()));
}
```

## Implementation skeleton

```rust
pub async fn buildauthorizationurl(
    auth_server_url: &str,
    client_id: &str,
    response_type: &str,
    redirect_uri: &str,
    state: &str,
    nonce: Option<&str>,
    prompt: Option<&str>,
    scope: Option<Vec<&str>>,
    max_age: Option<u32>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
