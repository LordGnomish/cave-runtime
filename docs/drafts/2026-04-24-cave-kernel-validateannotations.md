---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/api/validation/generic.go
upstream_fn: ValidateAnnotations
status: draft
tier: 1
created_at: 2026-04-24T16:30:06.135791+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/api/validation/generic.go` → `ValidateAnnotations`

## Failing test

```rust
#[tokio::test]
async fn test_validate_annotations() {
    use cave_kernel::validateannotations;
    use std::collections::BTreeMap;

    // Valid annotations: short keys, valid values
    let valid_annotations: BTreeMap<String, String> = [
        ("app.kubernetes.io/name", "my-app"),
        ("version", "v1.0.0"),
        ("kubernetes.io/hostname", "node-1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let errors = validateannotations(&valid_annotations, "metadata.annotations");
    assert!(errors.is_empty(), "Valid annotations should produce no errors");

    // Invalid: key too long (> 63 chars)
    let long_key = "a".repeat(64);
    let mut invalid_annotations = valid_annotations.clone();
    invalid_annotations.insert(long_key, "value");
    let errors = validateannotations(&invalid_annotations, "metadata.annotations");
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| e.contains("must be no more than 63 characters")));

    // Invalid: key contains invalid characters (e.g., uppercase)
    let mut invalid_annotations = valid_annotations.clone();
    invalid_annotations.insert("App.Name".to_string(), "value");
    let errors = validateannotations(&invalid_annotations, "metadata.annotations");
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| e.contains("must be a DNS-1123 label")));

    // Invalid: value too long (> 256 chars)
    let long_value = "x".repeat(257);
    let mut invalid_annotations = valid_annotations.clone();
    invalid_annotations.insert("custom.io/long-value".to_string(), long_value);
    let errors = validateannotations(&invalid_annotations, "metadata.annotations");
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| e.contains("must be no more than 256 characters")));

    // Invalid: key starts with invalid prefix (k8s.io/* is reserved for core Kubernetes)
    let mut invalid_annotations = valid_annotations.clone();
    invalid_annotations.insert("k8s.io/my-key".to_string(), "value");
    let errors = validateannotations(&invalid_annotations, "metadata.annotations");
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| e.contains("reserved prefix")));
}
```

## Implementation skeleton

```rust
pub fn validateannotations(
    annotations: &std::collections::BTreeMap<String, String>,
    field_path: &str,
) -> Vec<String> {
    todo!("Tier 2")
}
```
