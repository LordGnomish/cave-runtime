---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/lease/lease.go
upstream_fn: TTLWithLeader
status: draft
tier: 1
created_at: 2026-04-24T17:18:41.324867+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/lease/lease.go` → `TTLWithLeader`

## Failing test

```rust
#[tokio::test]
async fn test_ttlwithleader_basic() {
    use std::time::{Duration, Instant};
    use cave_etcd::lease::{Lease, LeaseId};
    use cave_etcd::ttlwithleader::ttlwithleader;

    // Create a mock lease with ID 1, TTL 2 seconds
    let lease = Lease {
        id: LeaseId(1),
        ttl: 2,
        remaining_ttl: 2,
        items: Vec::new(),
    };

    // Start TTL tracking with leader election simulation
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    let lease_clone = lease.clone();
    let handle = tokio::spawn(async move {
        ttlwithleader(lease_clone, tx).await
    });

    // Wait a bit to let the TTL countdown start
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify lease is still valid (not expired yet)
    assert_eq!(lease.remaining_ttl, 2);

    // Wait for TTL to expire (2 seconds total)
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Send a signal to stop the TTL loop
    drop(rx);

    // Wait for the task to finish
    let _ = handle.await;

    // After expiration, remaining_ttl should be <= 0
    // (Note: actual implementation may set it to 0 or negative depending on timing)
    assert!(lease.remaining_ttl <= 0, "Lease should be expired");
}
```

## Implementation skeleton

```rust
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

pub struct Lease {
    pub id: LeaseId,
    pub ttl: i64,
    pub remaining_ttl: i64,
    pub items: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LeaseId(pub u64);

pub async fn ttlwithleader(
    mut lease: Lease,
    _leader_tx: mpsc::Sender<Lease>,
) {
    // TODO: Implement leader election and TTL management logic
    // This is a placeholder for Tier 2 implementation
    todo!("Tier 2");
}
```
