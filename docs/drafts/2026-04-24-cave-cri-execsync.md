---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: ExecSync
status: draft
tier: 1
created_at: 2026-04-24T18:16:10.658107+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `ExecSync`

## Failing test

```rust
#[tokio::test]
async fn test_execsync() {
    use cave_cri::{ExecSyncRequest, ExecSyncResponse};
    use std::time::Duration;

    let request = ExecSyncRequest {
        container_id: "container-123".to_string(),
        cmd: vec!["echo".to_string(), "hello".to_string()],
        timeout: Some(5), // seconds
        stdin: false,
        stdout: true,
        stderr: true,
    };

    let response = cave_cri::execsync(request).await;

    match response {
        Ok(resp) => {
            assert_eq!(resp.exit_code, 0);
            assert_eq!(resp.stdout, "hello\n");
            assert_eq!(resp.stderr, "");
            assert!(resp.exec_id.is_empty());
        }
        Err(e) => panic!("execsync failed: {}", e),
    }
}
```

## Implementation skeleton

```rust
pub async fn execsync(_request: ExecSyncRequest) -> Result<ExecSyncResponse, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
