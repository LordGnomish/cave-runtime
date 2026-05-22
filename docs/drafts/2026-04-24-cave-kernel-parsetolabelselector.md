---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/apis/meta/v1/helpers.go
upstream_fn: ParseToLabelSelector
status: draft
tier: 1
created_at: 2026-04-24T16:30:57.676926+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/apis/meta/v1/helpers.go` → `ParseToLabelSelector`

## Failing test

```rust
#[tokio::test]
async fn test_parsetolabelselector() {
    use cave_kernel::parsetolabelselector;
    use std::collections::HashMap;

    // Valid selector: app=nginx,env!=prod
    let selector_str = "app=nginx,env!=prod";
    let result = parsetolabelselector(selector_str).await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector.match_labels.get("app"), Some(&"nginx".to_string()));
    assert_eq!(selector.match_expressions.len(), 1);
    let expr = &selector.match_expressions[0];
    assert_eq!(expr.key, "env");
    assert_eq!(expr.operator, "NotIn");
    assert_eq!(expr.values, vec!["prod".to_string()]);

    // Valid selector with empty values (should fail)
    let invalid_selector = "app=,env";
    let result = parsetolabelselector(invalid_selector).await;
    assert!(result.is_err());

    // Valid selector with multiple expressions
    let selector_str = "tier=frontend,tier!=backend,version in (v1,v2)";
    let result = parsetolabelselector(selector_str).await;
    assert!(result.is_ok());
    let selector = result.unwrap();
    assert_eq!(selector.match_labels.len(), 1);
    assert_eq!(selector.match_labels.get("tier"), Some(&"frontend".to_string()));
    assert_eq!(selector.match_expressions.len(), 2);
    let expr1 = &selector.match_expressions[0];
    assert_eq!(expr1.key, "tier");
    assert_eq!(expr1.operator, "NotIn");
    assert_eq!(expr1.values, vec!["backend".to_string()]);
    let expr2 = &selector.match_expressions[1];
    assert_eq!(expr2.key, "version");
    assert_eq!(expr2.operator, "In");
    assert_eq!(expr2.values, vec!["v1".to_string(), "v2".to_string()]);
}
```

## Implementation skeleton

```rust
use std::collections::HashMap;
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct LabelSelector {
    pub match_labels: HashMap<String, String>,
    pub match_expressions: Vec<LabelSelectorRequirement>,
}

#[derive(Debug, Clone)]
pub struct LabelSelectorRequirement {
    pub key: String,
    pub operator: String,
    pub values: Vec<String>,
}

pub async fn parsetolabelselector(selector_str: &str) -> Result<LabelSelector> {
    todo!("Tier 2")
}
```
