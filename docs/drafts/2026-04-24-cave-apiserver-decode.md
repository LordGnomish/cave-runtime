---
crate: cave-apiserver
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/runtime/serializer/json/json.go
upstream_fn: Decode
status: draft
tier: 1
created_at: 2026-04-24T16:38:06.611353+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/runtime/serializer/json/json.go` → `Decode`

## Failing test

```rust
#[tokio::test]
async fn test_decode_success_and_error_cases() {
    use cave_apiserver::decode;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestObject {
        api_version: String,
        kind: String,
        metadata: BTreeMap<String, String>,
        spec: Option<String>,
    }

    // Valid JSON object
    let valid_json = r#"{"api_version":"v1","kind":"ConfigMap","metadata":{"name":"test"},"spec":"data"}"#;
    let decoded: TestObject = decode(valid_json.as_bytes()).await.unwrap();
    assert_eq!(decoded.api_version, "v1");
    assert_eq!(decoded.kind, "ConfigMap");
    assert_eq!(decoded.metadata.get("name"), Some(&"test".to_string()));
    assert_eq!(decoded.spec, Some("data".to_string()));

    // Invalid JSON
    let invalid_json = r#"{"api_version":"v1","kind":"ConfigMap","metadata":{invalid}"#;
    let result = decode(invalid_json.as_bytes()).await;
    assert!(result.is_err());

    // Valid JSON but mismatched type (missing required field)
    let incomplete_json = r#"{"api_version":"v1"}"#;
    let result = decode(incomplete_json.as_bytes()).await;
    assert!(result.is_err());
}
```

## Implementation skeleton

```rust
pub async fn decode<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
