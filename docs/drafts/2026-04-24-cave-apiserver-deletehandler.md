---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/endpoints/handlers/delete.go
upstream_fn: deleteHandler
status: draft
tier: 1
created_at: 2026-04-24T17:27:28.768981+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/endpoints/handlers/delete.go` → `deleteHandler`

## Failing test

```rust
#[tokio::test]
async fn test_deletehandler_success() {
    use cave_apiserver::request::RequestInfo;
    use cave_apiserver::response::{Response, ResponseWriter};
    use cave_apiserver::storage::{StorageObject, StorageContext};
    use cave_apiserver::etcd::EtcdStorage;
    use cave_apiserver::apierror::ApiError;
    use cave_apiserver::types::{ObjectMeta, DeleteOptions};
    use cave_apiserver::runtime::deletehandler;
    use std::sync::Arc;
    use http::StatusCode;
    use bytes::Bytes;

    // Setup mock storage
    let storage = Arc::new(EtcdStorage::new("/test-prefix"));
    
    // Create a test object
    let obj = StorageObject::Pod {
        metadata: ObjectMeta {
            name: Some("test-pod".to_string()),
            namespace: Some("default".to_string()),
            uid: Some("uid-123".to_string()),
            resource_version: Some("100".to_string()),
            ..Default::default()
        },
        spec: Default::default(),
        status: Default::default(),
    };

    // Store the object
    storage.create("pods/default/test-pod", &obj).await.unwrap();

    // Prepare request info
    let req_info = RequestInfo {
        api_version: "v1".to_string(),
        resource: "pods".to_string(),
        subresource: None,
        name: Some("test-pod".to_string()),
        namespace: Some("default".to_string()),
        verb: "delete".to_string(),
        ..Default::default()
    };

    // Prepare delete options
    let delete_options = DeleteOptions {
        grace_period_seconds: Some(0),
        preconditions: None,
        orphan_dependents: Some(false),
        propagation_policy: Some("Background".to_string()),
        dry_run: vec![],
    };

    // Mock response writer
    struct MockWriter {
        status: Option<StatusCode>,
        body: Option<Bytes>,
    }

    impl ResponseWriter for MockWriter {
        fn status(&mut self, status: StatusCode) {
            self.status = Some(status);
        }

        fn body(&mut self, body: Bytes) {
            self.body = Some(body);
        }
    }

    let mut writer = MockWriter {
        status: None,
        body: None,
    };

    // Call deletehandler
    let ctx = StorageContext {
        storage: storage.clone(),
        resource: "pods".to_string(),
        namespace: "default".to_string(),
        name: "test-pod".to_string(),
    };

    deletehandler(&ctx, &req_info, &delete_options, &mut writer).await;

    // Asserts
    assert_eq!(writer.status.unwrap(), StatusCode::OK);
    let body_str = String::from_utf8_lossy(&writer.body.unwrap());
    assert!(body_str.contains("\"kind\":\"Status\""));
    assert!(body_str.contains("\"apiVersion\":\"v1\""));
    assert!(body_str.contains("\"status\":\"Success\""));
    assert!(body_str.contains("\"code\":200"));
}
```

## Implementation skeleton

```rust
pub async fn deletehandler(
    ctx: &StorageContext,
    req_info: &RequestInfo,
    options: &DeleteOptions,
    writer: &mut impl ResponseWriter,
) {
    todo!("Tier 2")
}
```
