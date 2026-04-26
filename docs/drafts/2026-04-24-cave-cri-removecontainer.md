---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: RemoveContainer
status: draft
tier: 1
created_at: 2026-04-24T17:30:23.258032+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `RemoveContainer`

## Failing test

```rust
#[tokio::test]
async fn test_remove_container() {
    use cave_cri::{RemoveContainerRequest, RemoveContainerResponse};
    use cave_cri::client::CRIClient;
    use std::time::Duration;

    // Mock client that captures the request and returns a successful response
    struct MockClient {
        expected_container_id: String,
    }

    impl CRIClient for MockClient {
        async fn remove_container(
            &self,
            req: RemoveContainerRequest,
        ) -> Result<RemoveContainerResponse, Box<dyn std::error::Error + Send + Sync>> {
            assert_eq!(req.container_id, self.expected_container_id);
            assert_eq!(req.timeout, Some(Duration::from_secs(30)));
            Ok(RemoveContainerResponse {})
        }
    }

    let client = MockClient {
        expected_container_id: "container-123".to_string(),
    };

    let result = cave_cri::removecontainer(client, "container-123", Some(Duration::from_secs(30))).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().to_string(), "RemoveContainerResponse {{ }}");
}
```

## Implementation skeleton

```rust
pub async fn removecontainer<C: CRIClient>(
    client: C,
    container_id: impl Into<String>,
    timeout: Option<std::time::Duration>,
) -> Result<RemoveContainerResponse, Box<dyn std::error::Error + Send + Sync>> {
    let req = RemoveContainerRequest {
        container_id: container_id.into(),
        timeout,
    };
    client.remove_container(req).await
}
```
