---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/registry/generic/rest/create.go
upstream_fn: Create
status: draft
tier: 1
created_at: 2026-04-24T17:19:35.977985+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/registry/generic/rest/create.go` → `Create`

## Failing test

```rust
#[tokio::test]
async fn test_create_succeeds_with_valid_object() {
    use cave_apiserver::storage::Storage;
    use cave_apiserver::rest::create;
    use cave_types::api::core::v1::Pod;
    use cave_types::api::meta::v1::{ObjectMeta, OwnerReference};
    use cave_types::uid::Uid;
    use cave_types::timestamp::Timestamp;
    use std::collections::HashMap;
    use uuid::Uuid;

    // Setup mock storage
    let storage = Storage::new_in_memory();
    
    // Create a test pod
    let uid = Uuid::new_v4();
    let now = Timestamp::now();
    let pod = Pod {
        metadata: ObjectMeta {
            name: Some("test-pod".to_string()),
            namespace: Some("default".to_string()),
            uid: Uid::from(uid),
            resource_version: None,
            creation_timestamp: Some(now),
            deletion_timestamp: None,
            owner_references: Some(vec![OwnerReference {
                api_version: "v1".to_string(),
                kind: "Pod".to_string(),
                name: "test-pod".to_string(),
                uid: uid,
                controller: Some(true),
                block_owner_deletion: Some(true),
            }]),
            labels: Some(HashMap::from([("app".to_string(), "test".to_string())])),
            annotations: Some(HashMap::new()),
            finalizers: None,
            managed_fields: None,
        },
        spec: None,
        status: None,
    };

    // Call create
    let result = create(&storage, "pods", "default", pod.clone()).await;

    // Assert success
    assert!(result.is_ok(), "create should succeed");
    let created_pod = result.unwrap();
    
    // Verify returned pod has system-assigned fields populated
    assert!(created_pod.metadata.resource_version.is_some());
    assert!(created_pod.metadata.creation_timestamp.is_some());
    assert_eq!(created_pod.metadata.name, Some("test-pod".to_string()));
    assert_eq!(created_pod.metadata.namespace, Some("default".to_string()));
    assert_eq!(created_pod.metadata.uid, created_pod.metadata.uid); // same uid
}
```

## Implementation skeleton

```rust
pub async fn create<S, T>(
    storage: &S,
    resource: &str,
    namespace: &str,
    obj: T,
) -> Result<T, anyhow::Error>
where
    S: Storage,
    T: cave_types::api::meta::v1::Object + Send + Sync,
{
    todo!("Tier 2")
}
```
