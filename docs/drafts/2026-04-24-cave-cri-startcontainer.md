---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: StartContainer
status: draft
tier: 1
created_at: 2026-04-24T17:29:43.087077+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `StartContainer`

## Failing test

```rust
#[tokio::test]
async fn test_startcontainer_success() {
    use cave_cri::{StartContainerRequest, StartContainerResponse};
    use std::collections::HashMap;

    // Simulate a realistic container start request
    let request = StartContainerRequest {
        container_id: "container-123".to_string(),
        options: Some(HashMap::from([
            ("env".to_string(), "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string()),
            ("working_dir".to_string(), "/app".to_string()),
        ])),
    };

    // Mock the runtime service (in real code, this would be a gRPC client or internal service)
    // For testing, we'll assume the function is idempotent and returns a success response
    let result = startcontainer(request).await;

    assert!(result.is_ok());
    let response = result.unwrap();
    assert_eq!(response.container_id, "container-123");
    assert!(response.started_at > 0);
}
```

## Implementation skeleton

```rust
pub async fn startcontainer(
    _request: StartContainerRequest,
) -> Result<StartContainerResponse, anyhow::Error> {
    todo!("Tier 2")
}
```
