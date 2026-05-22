---
crate: cave-scheduler
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/scheduler/core/generic_scheduler.go
upstream_fn: selectHost
status: draft
tier: 1
created_at: 2026-04-24T18:17:40.354003+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/scheduler/core/generic_scheduler.go` → `selectHost`

## Failing test

```rust
#[tokio::test]
async fn test_selecthost_selects_best_node() {
    use cave_scheduler::{NodeInfo, PodInfo, ScoredNode, SchedulerError};
    use std::collections::HashMap;

    // Create mock nodes with varying scores
    let nodes = vec![
        NodeInfo {
            name: "node-1".to_string(),
            cpu_capacity: 1000,
            memory_capacity: 2048,
            allocatable_cpu: 900,
            allocatable_memory: 1800,
            labels: HashMap::new(),
            taints: vec![],
        },
        NodeInfo {
            name: "node-2".to_string(),
            cpu_capacity: 2000,
            memory_capacity: 4096,
            allocatable_cpu: 1800,
            allocatable_memory: 3600,
            labels: HashMap::new(),
            taints: vec![],
        },
        NodeInfo {
            name: "node-3".to_string(),
            cpu_capacity: 500,
            memory_capacity: 1024,
            allocatable_cpu: 400,
            allocatable_memory: 800,
            labels: HashMap::new(),
            taints: vec![],
        },
    ];

    let pod = PodInfo {
        name: "test-pod".to_string(),
        namespace: "default".to_string(),
        cpu_request: 200,
        memory_request: 400,
        node_selector: HashMap::new(),
        affinity: None,
        tolerations: vec![],
    };

    // Simulate scoring: node-2 should get highest score due to ample resources
    let result = cave_scheduler::selecthost(&nodes, &pod).await;

    match result {
        Ok(scored_nodes) => {
            assert!(!scored_nodes.is_empty(), "Should return at least one scored node");
            assert_eq!(scored_nodes[0].name, "node-2", "node-2 should be top scorer");
            assert!(scored_nodes[0].score >= scored_nodes[1].score);
            assert!(scored_nodes[1].score >= scored_nodes[2].score);
        }
        Err(e) => panic!("selecthost failed: {:?}", e),
    }
}
```

## Implementation skeleton

```rust
pub async fn selecthost(
    nodes: &[NodeInfo],
    pod: &PodInfo,
) -> Result<Vec<ScoredNode>, SchedulerError> {
    todo!("Tier 2")
}
```
