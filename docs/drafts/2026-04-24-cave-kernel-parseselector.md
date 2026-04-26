---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/fields/fields.go
upstream_fn: ParseSelector
status: draft
tier: 1
created_at: 2026-04-24T16:31:46.822786+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/fields/fields.go` → `ParseSelector`

## Failing test

```rust
#[tokio::test]
async fn test_parseselector() {
    use cave_kernel::parseselector;
    
    // Valid selector: key=value
    let result = parseselector("app=nginx").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector.len(), 1);
    assert_eq!(selector[0].key, "app");
    assert_eq!(selector[0].operator, "Equals");
    assert_eq!(selector[0].values, vec!["nginx"]);

    // Valid selector: key!=value
    let result = parseselector("zone!=us-west-1").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector[0].operator, "NotEquals");
    assert_eq!(selector[0].values, vec!["us-west-1"]);

    // Valid selector: key in (a,b,c)
    let result = parseselector("region in (us-east-1,us-west-2)").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector[0].operator, "In");
    assert_eq!(selector[0].values, vec!["us-east-1", "us-west-2"]);

    // Valid selector: key notin (x,y)
    let result = parseselector("disktype notin (ssd,hdd)").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector[0].operator, "NotIn");
    assert_eq!(selector[0].values, vec!["ssd", "hdd"]);

    // Valid selector: key exists
    let result = parseselector("env").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector[0].operator, "Exists");

    // Valid selector: key does not exist
    let result = parseselector("!env").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector[0].operator, "DoesNotExist");

    // Multiple selectors separated by commas
    let result = parseselector("app=nginx,env=prod").await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector.len(), 2);
    assert_eq!(selector[0].key, "app");
    assert_eq!(selector[1].key, "env");

    // Invalid: empty selector
    let result = parseselector("").await;
    assert!(result.is_err());

    // Invalid: malformed in clause
    let result = parseselector("region in (us-east-1").await;
    assert!(result.is_err());

    // Invalid: invalid operator
    let result = parseselector("app~nginx").await;
    assert!(result.is_err());
}
```

## Implementation skeleton

```rust
pub async fn parseselector(selector: &str) -> Result<Vec<SelectorField>, String> {
    todo!("Tier 2")
}
```
