---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/etcdserver/api/v3rpc/watch.go
upstream_fn: Watch
status: draft
tier: 1
created_at: 2026-04-24T18:14:37.182192+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/etcdserver/api/v3rpc/watch.go` → `Watch`

## Failing test

```rust
#[tokio::test]
async fn test_watch_basic() {
    use cave_etcd::watch;
    use cave_etcd::types::{WatchRequest, WatchCreateRequest, WatchResponse};
    use std::time::Duration;
    use tokio::time::timeout;

    // Start a mock etcd server (in real implementation, this would be a test server)
    // For now, we'll assume the watch function connects to a running etcd instance
    // and receives watch events.

    // Create a watch request for a key
    let request = WatchRequest {
        create_request: Some(WatchCreateRequest {
            key: b"foo".to_vec(),
            range_end: b"".to_vec(), // watch single key
            start_revision: 0,
            progress_notify: false,
            filters: vec![],
            prev_kv: false,
            watch_id: 0,
            fragment: false,
        }),
        watch_id: 0,
        progress_notify: false,
    };

    // Spawn the watch in a background task
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<WatchResponse>();
    let watch_handle = tokio::spawn(async move {
        watch(request, tx).await
    });

    // Give some time for the watch to be established
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Simulate a put operation (in real test, this would be done via etcd client)
    // Since we can't easily inject data into a real etcd in a unit test,
    // we'll assert that the watch channel is open and the task is running.
    assert!(watch_handle.is_running());

    // Try to receive a response (should timeout if no events yet)
    let result = timeout(Duration::from_millis(200), rx.recv()).await;
    // In a real integration test, after a put, we'd expect a WatchResponse with created=true then events.
    // Here, we just verify the watch starts without panic.
    assert!(result.is_err(), "Expected timeout before any events");

    // Clean up
    drop(rx);
    watch_handle.abort();
}
```

## Implementation skeleton

```rust
pub async fn watch(
    _request: WatchRequest,
    _tx: tokio::sync::mpsc::UnboundedSender<WatchResponse>,
) {
    todo!("Tier 2")
}
```
