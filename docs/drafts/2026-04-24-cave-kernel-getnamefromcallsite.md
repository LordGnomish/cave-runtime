---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/util/naming/from_stack.go
upstream_fn: GetNameFromCallsite
status: draft
tier: 1
created_at: 2026-04-24T16:31:14.792279+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/util/naming/from_stack.go` → `GetNameFromCallsite`

## Failing test

```rust
#[tokio::test]
async fn test_getnamefromcallsite() {
    use std::panic::AssertUnwindSafe;

    // Helper to capture the function name from a specific call site
    fn inner_function() -> String {
        getnamefromcallsite(0)
    }

    fn outer_function() -> String {
        inner_function()
    }

    // Test basic functionality: should extract function name from call stack
    let result = outer_function();
    assert_eq!(result, "inner_function");

    // Test with panic safety (ensure no panics on valid inputs)
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        getnamefromcallsite(1)
    })).unwrap();
    assert_eq!(result, "test_getnamefromcallsite");

    // Test with out-of-bounds frame (should return empty string or fallback)
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        getnamefromcallsite(100)
    })).unwrap();
    assert!(result.is_empty() || result == "test_getnamefromcallsite");
}
```

## Implementation skeleton

```rust
pub fn getnamefromcallsite(skip: usize) -> String {
    todo!("Tier 2")
}
```
