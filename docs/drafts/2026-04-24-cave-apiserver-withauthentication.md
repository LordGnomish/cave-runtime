---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/endpoints/filters/authentication.go
upstream_fn: WithAuthentication
status: draft
tier: 1
created_at: 2026-04-24T18:15:20.836532+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/endpoints/filters/authentication.go` → `WithAuthentication`

## Failing test

```rust
#[tokio::test]
async fn with_authentication_sets_auth_info_on_success() {
    use cave_apiserver::authentication::AuthenticationInfo;
    use cave_apiserver::filters::withauthentication;
    use http::{Request, StatusCode};
    use http_body::Body as HttpBody;
    use hyper::Body;
    use std::sync::Arc;

    // Mock authenticator that always succeeds
    struct MockAuthenticator;
    impl cave_apiserver::authentication::Authenticator for MockAuthenticator {
        fn authenticate(
            &self,
            _headers: &http::HeaderMap,
        ) -> Result<Option<AuthenticationInfo>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(Some(AuthenticationInfo {
                user: "test-user".to_string(),
                groups: vec!["system:authenticated".to_string()],
                extra: std::collections::HashMap::new(),
            }))
        }
    }

    let authenticator = Arc::new(MockAuthenticator);
    let handler = withauthentication(authenticator, |req: Request<Body>| async {
        // Extract auth info from request extensions
        let auth_info = req
            .extensions()
            .get::<AuthenticationInfo>()
            .expect("AuthenticationInfo should be set");
        assert_eq!(auth_info.user, "test-user");
        assert!(auth_info.groups.contains(&"system:authenticated".to_string()));
        Ok::<_, std::convert::Infallible>(http::Response::new(Body::empty()))
    });

    let mut req = Request::builder()
        .uri("https://example.com/api/v1/pods")
        .header("Authorization", "Bearer valid-token")
        .body(Body::empty())
        .unwrap();

    // Simulate request processing
    let resp = handler(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
```

## Implementation skeleton

```rust
pub async fn withauthentication<F, B>(
    authenticator: std::sync::Arc<dyn crate::authentication::Authenticator>,
    next: F,
) -> http::Response<B>
where
    F: FnOnce(http::Request<http_body::Body>) -> futures::future::BoxFuture<'static, Result<http::Response<B>, std::convert::Infallible>>,
    B: http_body::Body<Data = bytes::Bytes> + Send + 'static,
{
    todo!("Tier 2")
}
```
