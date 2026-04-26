---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/labels/selector.go
upstream_fn: Matches
status: draft
tier: 1
created_at: 2026-04-24T16:29:07.089952+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/labels/selector.go` → `Matches`

## Failing test

```rust
#[tokio::test]
async fn test_matches() {
    use cave_kernel::matches;
    use std::collections::HashMap;

    // Test case 1: empty selector matches everything
    let selector = HashMap::new();
    let labels = HashMap::from([
        ("app".to_string(), "nginx".to_string()),
        ("version".to_string(), "1.19".to_string()),
    ]);
    assert!(matches(&selector, &labels));

    // Test case 2: exact match
    let selector = HashMap::from([("app".to_string(), "nginx".to_string())]);
    let labels = HashMap::from([
        ("app".to_string(), "nginx".to_string()),
        ("version".to_string(), "1.19".to_string()),
    ]);
    assert!(matches(&selector, &labels));

    // Test case 3: mismatch
    let selector = HashMap::from([("app".to_string(), "apache".to_string())]);
    let labels = HashMap::from([
        ("app".to_string(), "nginx".to_string()),
        ("version".to_string(), "1.19".to_string()),
    ]);
    assert!(!matches(&selector, &labels));

    // Test case 4: selector with multiple keys, all must match
    let selector = HashMap::from([
        ("app".to_string(), "nginx".to_string()),
        ("tier".to_string(), "frontend".to_string()),
    ]);
    let labels = HashMap::from([
        ("app".to_string(), "nginx".to_string()),
        ("version".to_string(), "1.19".to_string()),
    ]);
    assert!(!matches(&selector, &labels));

    // Test case 5: selector with extra keys in labels is OK
    let selector = HashMap::from([("app".to_string(), "nginx".to_string())]);
    let labels = HashMap::from([
        ("app".to_string(), "nginx".to_string()),
        ("tier".to_string(), "frontend".to_string()),
        ("version".to_string(), "1.19".to_string()),
    ]);
    assert!(matches(&selector, &labels));
}
```

## Implementation skeleton

```rust
pub fn matches(selector: &std::collections::HashMap<String, String>, labels: &std::collections::HashMap<String, String>) -> bool {
    todo!("Tier 2")
}
```
