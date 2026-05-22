---
crate: cave-cri
upstream_repo: kubernetes/cri-api
upstream_file: pkg/apis/runtime/v1/api.proto
upstream_fn: Attach
status: draft
tier: 1
created_at: 2026-04-24T18:16:43.088134+00:00
---

## Upstream reference

`kubernetes/cri-api` → `pkg/apis/runtime/v1/api.proto` → `Attach`

## Failing test

```rust
#[tokio::test]
async fn test_attach() {
    use cave_cri::{AttachRequest, AttachResponse};
    use std::io::{self, Read, Write};
    use tokio::io::{AsyncRead, AsyncWrite};
    use tokio::net::TcpStream;
    use tokio::sync::mpsc;

    // Create a mock terminal server that echoes back what it receives
    let server_addr = "127.0.0.1:0";
    let listener = tokio::net::TcpListener::bind(server_addr)
        .await
        .expect("Failed to bind server");
    let addr = listener.local_addr().unwrap();

    let server_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (mut reader, mut writer) = stream.into_split();
        let mut buf = [0u8; 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    writer.write_all(&buf[..n]).await.unwrap();
                    writer.flush().await.unwrap();
                }
                Err(_) => break,
            }
        }
    });

    // Prepare attach request
    let request = AttachRequest {
        container_id: "test-container-id".to_string(),
        stdin: true,
        stdout: true,
        stderr: false,
        tty: true,
        resize_channel: false,
    };

    // Simulate stdin/stdout streams
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(1);
    let (mut stdout_tx, stdout_rx) = tokio::io::duplex(1024);
    let (stdin_writer, mut stdin_reader) = tokio::io::duplex(1024);

    // Spawn stdin writer task
    let stdin_task = tokio::spawn(async move {
        stdin_tx.send(b"hello".to_vec()).await.unwrap();
        drop(stdin_tx);
        // Wait for echo
        let mut buf = [0u8; 5];
        stdin_reader.read_exact(&mut buf).await.unwrap();
        buf.to_vec()
    });

    // Perform attach
    let result = cave_cri::attach(&addr.to_string(), &request, stdin_writer, stdout_rx).await;

    // Assert success
    assert!(matches!(result, Ok(AttachResponse { .. })));

    // Wait for stdin task to complete and verify echo
    let echoed = stdin_task.await.unwrap();
    assert_eq!(echoed, b"hello");

    // Cancel server task
    server_task.abort();
}
```

## Implementation skeleton

```rust
pub async fn attach(
    endpoint: &str,
    request: &AttachRequest,
    stdin: impl AsyncWrite + Unpin + Send,
    stdout: impl AsyncRead + Unpin + Send,
) -> Result<AttachResponse, anyhow::Error> {
    todo!("Tier 2")
}
```
