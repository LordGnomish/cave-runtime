---
crate: cave-scheduler
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/scheduler/framework/plugins/podtopologyspread/filtering.go
upstream_fn: Filter
status: draft
tier: 1
created_at: 2026-04-24T17:35:41.309244+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/scheduler/framework/plugins/podtopologyspread/filtering.go` → `Filter`

## Failing test

```rust
#[tokio::test]
async fn test_filter_filters_nodes_based_on_topology_spread_constraints() {
    use cave_scheduler::filter;
    use cave_types::node::NodeInfo;
    use cave_types::pod::PodInfo;
    use k8s_openapi::api::core::v1::Pod;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
    use std::collections::HashMap;

    // Create a pod with a topology spread constraint
    let pod = Pod {
        metadata: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some("test-pod".to_string()),
            namespace: Some("default".to_string()),
            labels: Some(HashMap::from([
                ("app".to_string(), "web".to_string()),
            ])),
            ..Default::default()
        }),
        spec: Some({
            let mut spec = k8s_openapi::api::core::v1::PodSpec {
                topology_spread_constraints: Some(vec![
                    k8s_openapi::api::core::v1::TopologySpreadConstraint {
                        max_skew: 1,
                        topology_key: "kubernetes.io/hostname".to_string(),
                        when_unsatisfiable: "DoNotSchedule".to_string(),
                        label_selector: Some(LabelSelector {
                            match_labels: Some(HashMap::from([
                                ("app".to_string(), "web".to_string()),
                            ])),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }
                ]),
                ..Default::default()
            };
            spec.node_selector = Some(HashMap::new());
            spec
        }),
        ..Default::default()
    };

    let pod_info = PodInfo::new(pod);

    // Create two nodes: one with matching pods, one without
    let node1 = NodeInfo::new(
        "node-1".to_string(),
        HashMap::from([
            ("kubernetes.io/hostname".to_string(), "node-1".to_string()),
        ]),
        vec![
            PodInfo::new(Pod {
                metadata: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                    name: Some("other-pod".to_string()),
                    namespace: Some("default".to_string()),
                    labels: Some(HashMap::from([
                        ("app".to_string(), "web".to_string()),
                    ])),
                    ..Default::default()
                }),
                spec: Some(k8s_openapi::api::core::v1::PodSpec {
                    node_name: Some("node-1".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        ],
    );

    let node2 = NodeInfo::new(
        "node-2".to_string(),
        HashMap::from([
            ("kubernetes.io/hostname".to_string(), "node-2".to_string()),
        ]),
        vec![],
    );

    let nodes = vec![node1, node2];

    // Filter should exclude node-1 (would cause max_skew violation) and keep node-2
    let filtered = filter(&pod_info, &nodes).await;

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name(), "node-2");
}
```

## Implementation skeleton

```rust
pub async fn filter(pod_info: &PodInfo, nodes: &[NodeInfo]) -> Vec<NodeInfo> {
    todo!("Tier 2")
}
```
