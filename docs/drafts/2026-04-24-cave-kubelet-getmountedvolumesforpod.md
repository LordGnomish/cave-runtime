---
crate: cave-kubelet
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/kubelet/volumemanager/volume_manager.go
upstream_fn: GetMountedVolumesForPod
status: draft
tier: 1
created_at: 2026-04-24T17:30:44.783664+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/kubelet/volumemanager/volume_manager.go` → `GetMountedVolumesForPod`

## Failing test

```rust
#[tokio::test]
async fn test_getmountedvolumesforpod() {
    use cave_kubelet::volume_manager::VolumeManager;
    use cave_kubelet::volume_manager::volume_spec::VolumeSpec;
    use cave_kubelet::volume_manager::volume_state::VolumeState;
    use kube::api::ObjectMeta;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use uuid::Uuid;

    // Create a mock volume manager with some mounted volumes
    let pod_uid = Uuid::new_v4();
    let volume_name = "test-volume";
    let mount_path = PathBuf::from("/var/lib/kubelet/pods/test-pod/volumes/test-volume");
    
    let mut volume_states = HashMap::new();
    volume_states.insert(
        volume_name.to_string(),
        VolumeState {
            spec: VolumeSpec::PersistentVolumeClaim {
                claim_name: "test-pvc".to_string(),
                namespace: "default".to_string(),
            },
            mount_path: mount_path.clone(),
            is_ready: true,
        },
    );

    let volume_manager = VolumeManager {
        pod_uid,
        volume_states,
        ..Default::default()
    };

    // Call the function under test
    let volumes = volume_manager.getmountedvolumesforpod().await;

    // Assert expected behavior
    assert_eq!(volumes.len(), 1);
    let volume = &volumes[0];
    assert_eq!(volume.name, volume_name);
    assert_eq!(volume.mount_path, mount_path);
    assert!(volume.is_ready);
}
```

## Implementation skeleton

```rust
pub async fn getmountedvolumesforpod(&self) -> Vec<VolumeInfo> {
    todo!("Tier 2")
}
```
