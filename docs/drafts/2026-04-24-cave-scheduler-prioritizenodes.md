---
crate: cave-scheduler
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/scheduler/core/generic_scheduler.go
upstream_fn: prioritizeNodes
status: draft
tier: 1
created_at: 2026-04-24T18:18:23.132183+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/scheduler/core/generic_scheduler.go` → `prioritizeNodes`

## Failing test

```rust
#[tokio::test]
async fn test_prioritize_nodes() {
    use cave_scheduler::{PrioritizeNode, NodeInfo, PodInfo, Score};
    use std::collections::HashMap;

    // Create mock nodes with different scores
    let node1 = NodeInfo {
        name: "node-1".to_string(),
        labels: HashMap::from([
            ("kubernetes.io/hostname".to_string(), "node-1".to_string()),
        ]),
        allocatable: HashMap::from([
            ("cpu".to_string(), 4000),
            ("memory".to_string(), 8_000_000_000),
        ]),
        used: HashMap::from([
            ("cpu".to_string(), 1000),
            ("memory".to_string(), 2_000_000_000),
        ]),
    };

    let node2 = NodeInfo {
        name: "node-2".to_string(),
        labels: HashMap::from([
            ("kubernetes.io/hostname".to_string(), "node-2".to_string()),
        ]),
        allocatable: HashMap::from([
            ("cpu".to_string(), 8000),
            ("memory".to_string(), 16_000_000_000),
        ]),
        used: HashMap::from([
            ("cpu".to_string(), 2000),
            ("memory".to_string(), 4_000_000_000),
        ]),
    };

    let node3 = NodeInfo {
        name: "node-3".to_string(),
        labels: HashMap::from([
            ("kubernetes.io/hostname".to_string(), "node-3".to_string()),
        ]),
        allocatable: HashMap::from([
            ("cpu".to_string(), 2000),
            ("memory".to_string(), 4_000_000_000),
        ]),
        used: HashMap::from([
            ("cpu".to_string(), 1500),
            ("memory".to_string(), 3_500_000_000),
        ]),
    };

    let nodes = vec![node1, node2, node3];

    // Create a pod requesting 1 CPU and 2Gi memory
    let pod = PodInfo {
        name: "test-pod".to_string(),
        namespace: "default".to_string(),
        resources: HashMap::from([
            ("cpu".to_string(), 1000),
            ("memory".to_string(), 2_000_000_000),
        ]),
        ..Default::default()
    };

    // Mock prioritize node function: higher score for more available resources
    let prioritize_fn = |node: &NodeInfo, _pod: &PodInfo| -> Score {
        let cpu_avail = node.allocatable.get("cpu").copied().unwrap_or(0) - 
                        node.used.get("cpu").copied().unwrap_or(0);
        let mem_avail = node.allocatable.get("memory").copied().unwrap_or(0) - 
                        node.used.get("memory").copied().unwrap_or(0);
        Score::new((cpu_avail + mem_avail / 1_000_000) as i64, "resource_score")
    };

    let scores = cave_scheduler::prioritize_nodes(&nodes, &pod, prioritize_fn).await;

    // Verify results are sorted descending by score
    assert_eq!(scores.len(), 3);
    assert_eq!(scores[0].node.name, "node-2");
    assert_eq!(scores[1].node.name, "node-1");
    assert_eq!(scores[2].node.name, "node-3");

    // Verify scores are descending
    assert!(scores[0].score >= scores[1].score);
    assert!(scores[1].score >= scores[2].score);
}
```

## Implementation skeleton

```rust
pub async fn prioritize_nodes<F>(
    nodes: &[NodeInfo],
    pod: &PodInfo,
    prioritize_fn: F,
) -> Vec<ScheduledNode>
where
    F: Fn(&NodeInfo, &PodInfo) -> Score + Send + Sync,
{
    todo!("Tier 2")
}
```
