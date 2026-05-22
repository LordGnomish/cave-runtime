---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: StopContainer
status: draft
tier: 1
created_at: 2026-04-24T17:30:03.128709+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `StopContainer`

## Failing test

```rust
#[tokio::test]
async fn test_stopcontainer_success() {
    use cave_cri::{StopContainerRequest, StopContainerResponse};
    use cave_runtime_api::v1::runtime_service_client::RuntimeServiceClient;
    use cave_runtime_api::v1::{StopContainerRequest as GrpcRequest, StopContainerResponse as GrpcResponse};
    use std::time::Duration;
    use tokio::time::timeout;

    // Mock gRPC server behavior via in-process channel (simplified for test)
    let (tx, _rx) = tokio::sync::mpsc::channel::<GrpcRequest>(1);
    let client = RuntimeServiceClient::new(tx);

    let req = StopContainerRequest {
        container_id: "container-123".to_string(),
        timeout: Some(Duration::from_secs(10)),
    };

    // Simulate successful gRPC response
    let result = timeout(Duration::from_secs(5), async move {
        client.stop_container(req).await
    })
    .await;

    match result {
        Ok(Ok(response)) => {
            assert_eq!(response.container_id, "container-123");
        }
        Ok(Err(e)) => panic!("gRPC call failed: {}", e),
        Err(_) => panic!("timeout waiting for stop_container response"),
    }
}
```

## Implementation skeleton

```rust
pub async fn stopcontainer(
    _request: StopContainerRequest,
) -> Result<StopContainerResponse, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
