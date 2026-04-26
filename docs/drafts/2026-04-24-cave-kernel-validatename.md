---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/api/validation/generic.go
upstream_fn: ValidateName
status: draft
tier: 1
created_at: 2026-04-24T16:29:35.637661+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/api/validation/generic.go` → `ValidateName`

## Failing test

```rust
#[tokio::test]
async fn test_validatename() {
    use cave_kernel::validatename;
    
    // Valid names
    assert!(validatename("my-resource").is_ok());
    assert!(validatename("resource-123").is_ok());
    assert!(validatename("a").is_ok());
    assert!(validatename("resource.name").is_ok());
    assert!(validatename("resource_name").is_ok());
    
    // Invalid names: too long (> 253 chars)
    let long_name = "a".repeat(254);
    assert!(validatename(&long_name).is_err());
    
    // Invalid names: starts/ends with dash or dot
    assert!(validatename("-invalid").is_err());
    assert!(validatename("invalid-").is_err());
    assert!(validatename(".invalid").is_err());
    assert!(validatename("invalid.").is_err());
    
    // Invalid names: contains invalid characters
    assert!(validatename("invalid_name!").is_err());
    assert!(validatename("invalid@name").is_err());
    assert!(validatename("invalid#name").is_err());
    assert!(validatename("invalid name").is_err());
    
    // Invalid names: empty
    assert!(validatename("").is_err());
}
```

## Implementation skeleton

```rust
pub fn validatename(name: &str) -> Result<(), String> {
    todo!("Tier 2")
}
```
