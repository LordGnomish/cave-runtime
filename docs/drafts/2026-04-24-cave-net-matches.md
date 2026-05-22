---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/policy/selector.go
upstream_fn: Matches
status: draft
tier: 1
created_at: 2026-04-24T16:43:23.347140+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/policy/selector.go` → `Matches`

## Failing test

```rust
#[tokio::test]
async fn test_matches() {
    use cave_net::matches;
    use std::collections::HashSet;

    // Define selector with labels and namespace
    let selector = HashSet::from([
        ("app", "frontend"),
        ("version", "v1"),
    ]);
    let namespace = "production";

    // Test case 1: Pod with matching labels and namespace
    let pod_labels = HashSet::from([
        ("app", "frontend"),
        ("version", "v1"),
    ]);
    let pod_namespace = "production";
    assert!(matches(&selector, namespace, &pod_labels, pod_namespace));

    // Test case 2: Pod with matching labels but wrong namespace
    let pod_labels = HashSet::from([
        ("app", "frontend"),
        ("version", "v1"),
    ]);
    let pod_namespace = "staging";
    assert!(!matches(&selector, namespace, &pod_labels, pod_namespace));

    // Test case 3: Pod with missing label (should not match)
    let pod_labels = HashSet::from([
        ("app", "frontend"),
        ("tier", "web"),
    ]);
    let pod_namespace = "production";
    assert!(!matches(&selector, namespace, &pod_labels, pod_namespace));

    // Test case 4: Empty selector matches any pod in same namespace
    let empty_selector: HashSet<(&str, &str)> = HashSet::new();
    let pod_labels = HashSet::from([
        ("app", "backend"),
        ("version", "v2"),
    ]);
    let pod_namespace = "production";
    assert!(matches(&empty_selector, namespace, &pod_labels, pod_namespace));

    // Test case 5: Empty selector with mismatched namespace
    assert!(!matches(&empty_selector, namespace, &pod_labels, "development"));
}
```

## Implementation skeleton

```rust
pub fn matches(
    selector: &HashSet<(&str, &str)>,
    selector_namespace: &str,
    pod_labels: &HashSet<(&str, &str)>,
    pod_namespace: &str,
) -> bool {
    todo!("Tier 2")
}
```
