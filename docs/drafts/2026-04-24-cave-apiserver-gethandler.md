---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apiserver/pkg/endpoints/handlers/get.go
upstream_fn: getHandler
status: draft
tier: 1
created_at: 2026-04-24T17:25:16.006554+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apiserver/pkg/endpoints/handlers/get.go` → `getHandler`

## Failing test

```rust
#[tokio::test]
async fn test_gethandler_returns_resource_or_not_found() {
    use cave_apiserver::endpoints::gethandler;
    use cave_apiserver::request::RequestInfo;
    use cave_apiserver::storage::Storage;
    use cave_apiserver::types::{ObjectMeta, RawObject};
    use cave_runtime::api::core::v1::Pod;
    use cave_runtime::api::core::v1::ObjectReference;
    use cave_runtime::error::Error as CaveError;
    use cave_runtime::status::StatusCode;
    use std::sync::Arc;

    // Mock storage that returns a pod
    struct MockStorage {
        pod: Pod,
    }

    impl Storage for MockStorage {
        type Object = Pod;

        async fn get(
            &self,
            _namespace: &str,
            _name: &str,
            _options: &cave_runtime::api::meta::v1::GetOptions,
        ) -> Result<Self::Object, CaveError> {
            Ok(self.pod.clone())
        }
    }

    let pod = Pod {
        metadata: ObjectMeta {
            name: Some("test-pod".to_string()),
            namespace: Some("default".to_string()),
            ..Default::default()
        },
        spec: None,
        status: None,
    };

    let storage = Arc::new(MockStorage { pod: pod.clone() });

    let request_info = RequestInfo {
        namespace: Some("default".to_string()),
        name: "test-pod".to_string(),
        resource: "pods".to_string(),
        subresource: None,
        verb: "get".to_string(),
        api_group: Some("".to_string()),
        api_version: Some("v1".to_string()),
    };

    let result = gethandler(storage, &request_info, None).await;

    match result {
        Ok(obj) => {
            assert_eq!(obj.metadata.name, Some("test-pod".to_string()));
            assert_eq!(obj.metadata.namespace, Some("default".to_string()));
        }
        Err(e) => panic!("Expected success, got error: {:?}", e),
    }

    // Test not found case
    struct NotFoundStorage;

    impl Storage for NotFoundStorage {
        type Object = Pod;

        async fn get(
            &self,
            _namespace: &str,
            _name: &str,
            _options: &cave_runtime::api::meta::v1::GetOptions,
        ) -> Result<Self::Object, CaveError> {
            Err(CaveError::new(
                StatusCode::NOT_FOUND,
                "pod \"test-pod\" not found".to_string(),
            ))
        }
    }

    let storage = Arc::new(NotFoundStorage);
    let result = gethandler(storage, &request_info, None).await;

    match result {
        Err(e) => {
            assert_eq!(e.status_code(), StatusCode::NOT_FOUND);
            assert!(e.message().contains("not found"));
        }
        Ok(_) => panic!("Expected NOT_FOUND error, got success"),
    }
}
```

## Implementation skeleton

```rust
pub async fn gethandler<S: Storage>(
    storage: Arc<S>,
    request_info: &RequestInfo,
    options: Option<&cave_runtime::api::meta::v1::GetOptions>,
) -> Result<S::Object, CaveError>
where
    S::Object: RawObject,
{
    todo!("Tier 2")
}
```
