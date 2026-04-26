---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/endpointmanager/manager.go
upstream_fn: GetEndpoints
status: draft
tier: 1
created_at: 2026-04-24T18:19:02.033778+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/endpointmanager/manager.go` → `GetEndpoints`

## Failing test

```rust
#[tokio::test]
async fn test_getendpoints_returns_filtered_endpoints() {
    use cave_net::endpoint::{Endpoint, EndpointState, Identity};
    use cave_net::getendpoints::getendpoints;
    use std::collections::HashSet;

    // Create sample endpoints
    let endpoint1 = Endpoint {
        id: 1,
        state: EndpointState::Ready,
        identity: Identity::from(100),
        ipv4: "10.0.0.1".parse().unwrap(),
        ipv6: "fd00::1".parse().unwrap(),
        labels: HashSet::from(["id=foo".to_string(), "env=prod".to_string()]),
        ..Default::default()
    };

    let endpoint2 = Endpoint {
        id: 2,
        state: EndpointState::WaitingToRegister,
        identity: Identity::from(200),
        ipv4: "10.0.0.2".parse().unwrap(),
        ipv6: "fd00::2".parse().unwrap(),
        labels: HashSet::from(["id=bar".to_string(), "env=dev".to_string()]),
        ..Default::default()
    };

    let endpoint3 = Endpoint {
        id: 3,
        state: EndpointState::Ready,
        identity: Identity::from(300),
        ipv4: "10.0.0.3".parse().unwrap(),
        ipv6: "fd00::3".parse().unwrap(),
        labels: HashSet::from(["id=baz".to_string(), "env=prod".to_string()]),
        ..Default::default()
    };

    let all_endpoints = vec![endpoint1.clone(), endpoint2.clone(), endpoint3.clone()];

    // Test 1: no filters → return all ready endpoints
    let result = getendpoints(&all_endpoints, &Default::default()).await;
    assert_eq!(result.len(), 2);
    assert!(result.iter().any(|e| e.id == 1));
    assert!(result.iter().any(|e| e.id == 3));

    // Test 2: filter by state = Ready
    let filters = cave_net::getendpoints::Filters {
        state: Some(EndpointState::Ready),
        ..Default::default()
    };
    let result = getendpoints(&all_endpoints, &filters).await;
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|e| e.state == EndpointState::Ready));

    // Test 3: filter by label "env=prod"
    let filters = cave_net::getendpoints::Filters {
        labels: HashSet::from(["env=prod".to_string()]),
        ..Default::default()
    };
    let result = getendpoints(&all_endpoints, &filters).await;
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|e| e.labels.contains("env=prod")));

    // Test 4: filter by identity
    let filters = cave_net::getendpoints::Filters {
        identities: HashSet::from([Identity::from(200)]),
        ..Default::default()
    };
    let result = getendpoints(&all_endpoints, &filters).await;
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, 2);

    // Test 5: no matches
    let filters = cave_net::getendpoints::Filters {
        labels: HashSet::from(["nonexistent".to_string()]),
        ..Default::default()
    };
    let result = getendpoints(&all_endpoints, &filters).await;
    assert!(result.is_empty());
}
```

## Implementation skeleton

```rust
pub async fn getendpoints(
    endpoints: &[Endpoint],
    filters: &Filters,
) -> Vec<Endpoint> {
    todo!("Tier 2")
}
```
