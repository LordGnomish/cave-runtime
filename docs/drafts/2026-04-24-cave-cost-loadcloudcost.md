---
crate: cave-cost
upstream_repo: opencost/opencost
upstream_file: pkg/cloudcost/cloudcost.go
upstream_fn: LoadCloudCost
status: draft
tier: 1
created_at: 2026-04-24T18:26:56.845530+00:00
---

## Upstream reference

`opencost/opencost` → `pkg/cloudcost/cloudcost.go` → `LoadCloudCost`

## Failing test

```rust
#[tokio::test]
async fn test_loadcloudcost() {
    use cave_cost::{CloudCost, CloudCostLoader, CloudCostProvider, CloudCostQuery};
    use std::collections::HashMap;
    use std::time::{Duration, SystemTime};

    // Create a mock provider that returns known data
    struct MockProvider {
        costs: Vec<CloudCost>,
    }

    impl CloudCostProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn query(&self, _query: &CloudCostQuery) -> Result<Vec<CloudCost>, String> {
            Ok(self.costs.clone())
        }
    }

    let start = SystemTime::now() - Duration::from_secs(3600);
    let end = SystemTime::now();
    let query = CloudCostQuery {
        start,
        end,
        window: "1h".to_string(),
        namespace: Some("default".to_string()),
        pod: None,
        service: None,
        labels: HashMap::new(),
    };

    let costs = vec![
        CloudCost {
            provider_id: "aws:i-12345".to_string(),
            name: "web-server".to_string(),
            namespace: "default".to_string(),
            pod: Some("web-pod-abc".to_string()),
            service: Some("web-service".to_string()),
            start,
            end: start + Duration::from_secs(1800),
            cpu_cost: 0.05,
            memory_cost: 0.03,
            gpu_cost: 0.0,
            total_cost: 0.08,
            cpu_core_seconds: 3600.0,
            memory_byte_seconds: 7200.0,
            gpu_seconds: 0.0,
            labels: HashMap::from([
                ("app".to_string(), "web".to_string()),
                ("tier".to_string(), "frontend".to_string()),
            ]),
            annotations: HashMap::new(),
        },
        CloudCost {
            provider_id: "gcp:vm-67890".to_string(),
            name: "db-server".to_string(),
            namespace: "database".to_string(),
            pod: Some("db-pod-xyz".to_string()),
            service: Some("db-service".to_string()),
            start: start + Duration::from_secs(1800),
            end,
            cpu_cost: 0.12,
            memory_cost: 0.09,
            gpu_cost: 0.0,
            total_cost: 0.21,
            cpu_core_seconds: 1800.0,
            memory_byte_seconds: 3600.0,
            gpu_seconds: 0.0,
            labels: HashMap::from([
                ("app".to_string(), "db".to_string()),
                ("tier".to_string(), "backend".to_string()),
            ]),
            annotations: HashMap::new(),
        },
    ];

    let provider = Box::new(MockProvider { costs });
    let loader = CloudCostLoader::new(vec![provider]);

    let result = loader.load(&query).await;

    assert!(result.is_ok());
    let cloud_costs = result.unwrap();
    assert_eq!(cloud_costs.len(), 2);
    assert_eq!(cloud_costs[0].name, "web-server");
    assert_eq!(cloud_costs[0].namespace, "default");
    assert_eq!(cloud_costs[0].total_cost, 0.08);
    assert_eq!(cloud_costs[1].name, "db-server");
    assert_eq!(cloud_costs[1].namespace, "database");
    assert_eq!(cloud_costs[1].total_cost, 0.21);
}
```

## Implementation skeleton

```rust
pub async fn loadcloudcost(
    _query: &CloudCostQuery,
    _providers: Vec<Box<dyn CloudCostProvider>>,
) -> Result<Vec<CloudCost>, String> {
    todo!("Tier 2")
}
```
