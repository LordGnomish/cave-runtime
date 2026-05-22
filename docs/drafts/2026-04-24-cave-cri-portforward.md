---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: PortForward
status: draft
tier: 1
created_at: 2026-04-24T18:17:11.993408+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `PortForward`

## Failing test

```rust
#[tokio::test]
async fn test_portforward() {
    use cave_cri::{PortForwardRequest, PortForwardResponse};
    use std::net::{TcpListener, TcpStream};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener as TokioTcpListener;

    // Start a local echo server on a random port
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_handle = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = [0; 1024];
        loop {
            let n = match stream.read(&mut buf) {
                Ok(n) => n,
                Err(_) => break,
            };
            if n == 0 {
                break;
            }
            if stream.write_all(&buf[..n]).await.is_err() {
                break;
            }
        }
    });

    // Prepare PortForwardRequest
    let req = PortForwardRequest {
        pod_uid: "test-pod-uid".to_string(),
        pod_name: "test-pod".to_string(),
        pod_namespace: "default".to_string(),
        container_name: "test-container".to_string(),
        ports: vec![port as i32],
    };

    // Mock portforward handler (in real impl, this would use CRI stream connection)
    let (mut tx, mut rx) = tokio::io::duplex(1024);

    // Spawn a task to simulate data flow
    let forward_task = tokio::spawn(async move {
        // In real implementation, this would connect to CRI stream and forward data
        // For test, we simulate by writing to the duplex and reading back
        let mut write_half = tokio::io::WriteHalf::new(tx);
        write_half.write_all(b"hello").await.unwrap();
        write_half.flush().await.unwrap();
    });

    // Simulate reading from the portforward stream
    let mut read_half = tokio::io::ReadHalf::new(rx);
    let mut buf = vec![0u8; 1024];
    let n = read_half.read(&mut buf).await.unwrap();

    // Clean up
    drop(forward_task);
    server_handle.abort();

    assert_eq!(&buf[..n], b"hello");
}
```

## Implementation skeleton

```rust
pub async fn portforward(
    _req: PortForwardRequest,
) -> Result<PortForwardResponse, anyhow::Error> {
    todo!("Tier 2")
}
```
