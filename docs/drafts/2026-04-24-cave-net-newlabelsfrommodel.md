---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/labels/labels.go
upstream_fn: NewLabelsFromModel
status: draft
tier: 1
created_at: 2026-04-24T16:42:59.489236+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/labels/labels.go` → `NewLabelsFromModel`

## Failing test

```rust
#[tokio::test]
async fn test_newlabelsfrommodel() {
    use cave_net::model::k8s::K8sMetadata;
    use cave_net::labels::Label;
    use std::collections::HashMap;

    // Simulate K8s metadata with labels and source
    let metadata = K8sMetadata {
        name: "pod-xyz".to_string(),
        namespace: "default".to_string(),
        labels: HashMap::from([
            ("app".to_string(), "frontend".to_string()),
            ("tier".to_string(), "web".to_string()),
        ]),
        annotations: HashMap::new(),
        uid: "uid-123".to_string(),
        resource_version: "42".to_string(),
        generation: 1,
        creation_timestamp: 1700000000,
        labels_source: "k8s".to_string(),
    };

    let labels = cave_net::newlabelsfrommodel(&metadata).await;

    // Expect labels derived from K8s metadata
    assert_eq!(labels.len(), 2);
    assert!(labels.iter().any(|l| l.key == "app" && l.value == "frontend"));
    assert!(labels.iter().any(|l| l.key == "tier" && l.value == "web"));
}
```

## Implementation skeleton

```rust
pub async fn newlabelsfrommodel(metadata: &cave_net::model::k8s::K8sMetadata) -> Vec<Label> {
    todo!("Tier 2")
}
```
