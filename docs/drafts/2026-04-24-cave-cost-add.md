---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cost/allocation.go
upstream_fn: Add
status: draft
tier: 1
created_at: 2026-04-24T17:15:38.385050+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cost/allocation.go` → `Add`

## Failing test

```rust
#[tokio::test]
async fn test_add_allocation() {
    use cave_cost::{Allocation, Cost, Resource, TimeRange};
    use std::collections::HashMap;

    let range = TimeRange::new(
        chrono::Utc::now() - chrono::Duration::hours(1),
        chrono::Utc::now(),
    );

    let mut alloc1 = Allocation::new("alloc1", range.clone());
    alloc1.add_cost(Cost {
        cpu: 0.1,
        memory: 0.2,
        gpu: 0.0,
        storage: 0.05,
    });
    alloc1.add_resource(Resource::Cpu, 2.0);
    alloc1.add_resource(Resource::Memory, 4.0);
    alloc1.add_resource(Resource::Gpu, 0.0);
    alloc1.add_resource(Resource::Storage, 1.0);

    let mut alloc2 = Allocation::new("alloc2", range);
    alloc2.add_cost(Cost {
        cpu: 0.15,
        memory: 0.25,
        gpu: 0.0,
        storage: 0.05,
    });
    alloc2.add_resource(Resource::Cpu, 3.0);
    alloc2.add_resource(Resource::Memory, 6.0);
    alloc2.add_resource(Resource::Gpu, 0.0);
    alloc2.add_resource(Resource::Storage, 1.5);

    let result = cave_cost::add(&alloc1, &alloc2).await;

    assert_eq!(result.id(), "alloc1+alloc2");
    assert_eq!(result.cost().cpu, 0.25);
    assert_eq!(result.cost().memory, 0.45);
    assert_eq!(result.cost().gpu, 0.0);
    assert_eq!(result.cost().storage, 0.1);
    assert_eq!(result.resources().get(&Resource::Cpu), Some(&5.0));
    assert_eq!(result.resources().get(&Resource::Memory), Some(&10.0));
    assert_eq!(result.resources().get(&Resource::Gpu), Some(&0.0));
    assert_eq!(result.resources().get(&Resource::Storage), Some(&2.5));
}
```

## Implementation skeleton

```rust
pub async fn add(a: &Allocation, b: &Allocation) -> Allocation {
    todo!("Tier 2")
}
```
