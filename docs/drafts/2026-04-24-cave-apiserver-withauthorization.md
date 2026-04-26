---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/endpoints/filters/authorization.go
upstream_fn: WithAuthorization
status: draft
tier: 1
created_at: 2026-04-24T18:15:54.958593+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/endpoints/filters/authorization.go` → `WithAuthorization`

## Failing test

```rust
#[tokio::test]
async fn withauthorization_allows_subject_access_review_on_authorized_path() {
    use http::{Request, StatusCode};
    use cave_runtime::apiserver::filters::authorization::withauthorization;
    use cave_runtime::apiserver::authentication::user::DefaultUser;
    use cave_runtime::apiserver::authentication::AuthenticationInfo;
    use cave_runtime::apiserver::authorization::subject_access_review::SubjectAccessReview;
    use cave_runtime::apiserver::authorization::Authorizer;
    use cave_runtime::apiserver::request::RequestInfo;
    use std::sync::Arc;

    // Mock authorizer that always allows
    struct MockAuthorizer;
    impl Authorizer for MockAuthorizer {
        fn authorize(
            &self,
            _user: &dyn AuthenticationInfo,
            _request_info: &RequestInfo,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    // Mock authorizer that denies
    struct DenyingAuthorizer;
    impl Authorizer for DenyingAuthorizer {
        fn authorize(
            &self,
            _user: &dyn AuthenticationInfo,
            _request_info: &RequestInfo,
        ) -> Result<(), String> {
            Err("access denied".to_string())
        }
    }

    // Helper to build a request
    let make_request = |path: &str| {
        Request::builder()
            .uri(format!("https://example.com{}", path))
            .body(())
            .unwrap()
    };

    // Helper to extract response status
    let extract_status = |resp: http::Response<hyper::Body>| {
        resp.status()
    };

    // Test 1: authorized path + allowed authorizer → passes through
    let authz = Arc::new(MockAuthorizer);
    let handler = withauthorization(authz.clone(), |req: Request<hyper::Body>| async move {
        Ok(http::Response::builder()
            .status(StatusCode::OK)
            .body(hyper::Body::from("authorized"))
            .unwrap())
    });

    let req = make_request("/apis/authorization.k8s.io/v1/subjectaccessreviews");
    let resp = handler(req).await;
    assert_eq!(extract_status(resp), StatusCode::OK);

    // Test 2: unauthorized path + denied authorizer → returns 403
    let authz = Arc::new(DenyingAuthorizer);
    let handler = withauthorization(authz.clone(), |req: Request<hyper::Body>| async move {
        Ok(http::Response::builder()
            .status(StatusCode::OK)
            .body(hyper::Body::from("should not reach"))
            .unwrap())
    });

    let req = make_request("/api/v1/pods");
    let resp = handler(req).await;
    assert_eq!(extract_status(resp), StatusCode::FORBIDDEN);
}
```

## Implementation skeleton

```rust
pub async fn withauthorization<F, Fut>(
    authorizer: Arc<dyn Authorizer>,
    next: F,
) -> http::Response<hyper::Body>
where
    F: FnOnce(http::Request<hyper::Body>) -> Fut,
    Fut: std::future::Future<Output = Result<http::Response<hyper::Body>, String>>,
{
    todo!("Tier 2")
}
```
