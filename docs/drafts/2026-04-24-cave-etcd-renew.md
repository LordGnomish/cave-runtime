---
crate: cave-etcd
upstream_repo: etcd-io/etcd
upstream_file: server/lease/lease.go
upstream_fn: Renew
status: draft
tier: 1
created_at: 2026-04-24T17:19:07.863779+00:00
---

## Upstream reference

`etcd-io/etcd` → `server/lease/lease.go` → `Renew`

## Failing test

```rust
#[tokio::test]
async fn test_renew_extends_lease_ttl() {
    use cave_etcd::lease::{Lease, LeaseId, LeaseStore};
    use std::time::{Duration, Instant};
    use tokio::time;

    // Mock lease store that tracks TTL changes
    struct MockStore {
        leases: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<LeaseId, Lease>>>,
    }

    impl LeaseStore for MockStore {
        fn get_lease(&self, id: LeaseId) -> Option<Lease> {
            self.leases.lock().unwrap().get(&id).cloned()
        }

        fn update_lease_ttl(&self, id: LeaseId, ttl: i64) -> bool {
            let mut leases = self.leases.lock().unwrap();
            if let Some(lease) = leases.get_mut(&id) {
                lease.ttl = ttl;
                true
            } else {
                false
            }
        }
    }

    let store = std::sync::Arc::new(MockStore {
        leases: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    });

    // Insert a lease with initial TTL of 10 seconds, created 2 seconds ago
    let lease_id = LeaseId(42);
    let now = Instant::now();
    let initial_lease = Lease {
        id: lease_id,
        ttl: 10,
        created: now - Duration::from_secs(2),
        items: Vec::new(),
    };
    store.leases.lock().unwrap().insert(lease_id, initial_lease);

    // Renew the lease
    let renewed_ttl = cave_etcd::lease::renew(&store, lease_id, 10).await;

    // Assert the TTL was extended (should be ~8 + 10 = 18 seconds remaining)
    let leases = store.leases.lock().unwrap();
    let renewed_lease = leases.get(&lease_id).expect("lease should exist");
    assert_eq!(renewed_lease.ttl, 18);
    assert_eq!(renewed_ttl, 18);
}
```

## Implementation skeleton

```rust
pub async fn renew(store: &impl LeaseStore, id: LeaseId, ttl: i64) -> i64 {
    todo!("Tier 2")
}
```
