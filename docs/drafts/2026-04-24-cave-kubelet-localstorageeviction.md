---
crate: cave-kubelet
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/kubelet/eviction/eviction_manager.go
upstream_fn: localStorageEviction
status: draft
tier: 1
created_at: 2026-04-24T17:31:20.622164+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/kubelet/eviction/eviction_manager.go` → `localStorageEviction`

## Failing test

```rust
#[tokio::test]
async fn test_localstorageeviction_evicts_under_pressure() {
    use cave_kubelet::eviction::localstorageeviction;
    use cave_kubelet::eviction::resources::{ResourcePressure, ResourceUsage};
    use cave_kubelet::pod::PodStatus;
    use std::collections::HashMap;
    use std::time::Duration;

    // Simulate a node with 100Gi total storage, 95Gi used (95% utilization)
    let total_storage_bytes = 100 * 1024 * 1024 * 1024; // 100 GiB
    let used_storage_bytes = 95 * 1024 * 1024 * 1024;   // 95 GiB
    let available_storage_bytes = total_storage_bytes - used_storage_bytes;

    // Create mock pod statuses with associated volumes
    let mut pod_statuses = Vec::new();
    for i in 0..3 {
        let mut volumes = HashMap::new();
        volumes.insert(
            format!("volume-{}", i),
            cave_kubelet::pod::VolumeStatus {
                name: format!("volume-{}", i),
                size_bytes: Some(10 * 1024 * 1024 * 1024), // 10 GiB each
                used_bytes: Some(8 * 1024 * 1024 * 1024),  // 8 GiB used
            },
        );
        pod_statuses.push(PodStatus {
            metadata: cave_kubelet::pod::ObjectMeta {
                name: format!("pod-{}", i),
                namespace: "default".to_string(),
                uid: format!("uid-{}", i),
                ..Default::default()
            },
            phase: cave_kubelet::pod::PodPhase::Running,
            volumes,
            ..Default::default()
        });
    }

    // Set eviction thresholds: 90% threshold should trigger eviction
    let eviction_config = cave_kubelet::eviction::EvictionConfig {
        eviction_hard: HashMap::from([
            ("nodefs.available".to_string(), "10%".to_string()),
            ("imagefs.available".to_string(), "15%".to_string()),
        ]),
        ..Default::default()
    };

    // Run eviction manager
    let result = localstorageeviction(
        &pod_statuses,
        ResourcePressure {
            resource: cave_kubelet::eviction::resources::Resource::NodeFs,
            usage: ResourceUsage {
                total_bytes: total_storage_bytes,
                available_bytes,
                used_bytes,
                inodes_total: 1_000_000,
                inodes_free: 500_000,
                inodes_used: 500_000,
            },
        },
        &eviction_config,
    )
    .await;

    // Should select at least one pod for eviction (the one with largest unused volume space)
    assert!(result.evictions.len() > 0, "Expected at least one pod to be evicted under storage pressure");
    assert_eq!(result.evictions[0].reason, "NodeFsPressure");
}
```

## Implementation skeleton

```rust
pub async fn localstorageeviction(
    pod_statuses: &[PodStatus],
    pressure: ResourcePressure,
    config: &EvictionConfig,
) -> EvictionResult {
    todo!("Tier 2")
}
```
