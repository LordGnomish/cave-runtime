---
crate: cave-scheduler
upstream_repo: kubernetes/kubernetes
upstream_file: pkg/scheduler/framework/plugins/affinity/affinity.go
upstream_fn: Score
status: draft
tier: 1
created_at: 2026-04-24T17:35:05.782366+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `pkg/scheduler/framework/plugins/affinity/affinity.go` → `Score`

## Failing test

```rust
#[tokio::test]
async fn test_score_calculates_correct_affinity_scores() {
    use cave_scheduler::score;
    use std::collections::HashMap;

    // Node 1: matches 2 preferred terms, 1 required term
    let node1_labels = HashMap::from([
        ("zone".to_string(), "us-east-1a".to_string()),
        ("rack".to_string(), "1".to_string()),
    ]);
    let node1_name = "node-1".to_string();

    // Node 2: matches 1 preferred term, 0 required terms
    let node2_labels = HashMap::from([
        ("zone".to_string(), "us-east-1b".to_string()),
    ]);
    let node2_name = "node-2".to_string();

    // Node 3: no matches
    let node3_labels = HashMap::from([
        ("zone".to_string(), "eu-west-1a".to_string()),
    ]);
    let node3_name = "node-3".to_string();

    // Pod affinity terms
    let preferred_terms = vec![
        // Weight 30: zone = us-east-1*
        cave_scheduler::api::WeightedPreference {
            weight: 30,
            preference: cave_scheduler::api::LabelSelectorRequirement {
                key: "zone".to_string(),
                operator: cave_scheduler::api::LabelSelectorOperator::In,
                values: vec!["us-east-1a".to_string(), "us-east-1b".to_string()],
            },
        },
        // Weight 20: rack = 1
        cave_scheduler::api::WeightedPreference {
            weight: 20,
            preference: cave_scheduler::api::LabelSelectorRequirement {
                key: "rack".to_string(),
                operator: cave_scheduler::api::LabelSelectorOperator::In,
                values: vec!["1".to_string()],
            },
        },
    ];

    let required_terms = vec![
        // Must match zone = us-east-1a
        cave_scheduler::api::LabelSelectorRequirement {
            key: "zone".to_string(),
            operator: cave_scheduler::api::LabelSelectorOperator::In,
            values: vec!["us-east-1a".to_string()],
        },
    ];

    let node_list = vec![
        (node1_name.clone(), node1_labels),
        (node2_name.clone(), node2_labels),
        (node3_name.clone(), node3_labels),
    ];

    let scores = score(&node_list, &preferred_terms, &required_terms).await;

    // Node 1: matches required term (score +100), matches both preferred (30+20=50) → 150
    // Node 2: no required match (0), matches one preferred (30) → 30
    // Node 3: no matches → 0
    assert_eq!(scores.len(), 3);
    assert_eq!(scores.get(&node1_name), Some(&150));
    assert_eq!(scores.get(&node2_name), Some(&30));
    assert_eq!(scores.get(&node3_name), Some(&0));
}
```

## Implementation skeleton

```rust
pub async fn score(
    nodes: &[(String, HashMap<String, String>)],
    preferred_terms: &[WeightedPreference],
    required_terms: &[LabelSelectorRequirement],
) -> HashMap<String, i64> {
    todo!("Tier 2")
}
```
