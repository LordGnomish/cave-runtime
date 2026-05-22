---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/labels/selector.go
upstream_fn: Parse
status: draft
tier: 1
created_at: 2026-04-24T16:28:35.536125+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/labels/selector.go` → `Parse`

## Failing test

```rust
#[tokio::test]
async fn test_parse_selector() {
    use cave_kernel::parse;
    use std::collections::HashMap;

    // Valid selectors
    let inputs = vec![
        ("", vec![]),
        ("foo=bar", vec![("foo".to_string(), "bar".to_string())]),
        ("foo!=bar", vec![("foo".to_string(), "bar".to_string())]),
        ("foo in (a,b,c)", vec![("foo".to_string(), "a,b,c".to_string())]),
        ("foo notin (x,y,z)", vec![("foo".to_string(), "x,y,z".to_string())]),
        ("foo", vec![("foo".to_string(), "".to_string())]),
        ("foo!=,bar", vec![("foo".to_string(), "".to_string()), ("bar".to_string(), "".to_string())]),
        ("foo=bar,baz", vec![("foo".to_string(), "bar,baz".to_string())]),
    ];

    for (input, expected) in inputs {
        let result = parse(input).await;
        assert!(result.is_ok(), "Failed to parse '{}': {:?}", input, result);
        let selectors = result.unwrap();
        let actual: Vec<(String, String)> = selectors
            .iter()
            .map(|s| (s.key.clone(), s.values.join(",")))
            .collect();
        assert_eq!(actual, expected, "Mismatch for input '{}'", input);
    }

    // Invalid selectors
    let invalid_inputs = vec![
        "foo=bar,baz", // comma in value not allowed without parentheses
        "foo in ()",
        "foo notin (a)",
        "foo in (a,,b)",
        "foo=bar,baz,qux", // multiple commas without parens
        "foo=bar,",
        "=bar",
        "foo=",
    ];

    for input in invalid_inputs {
        let result = parse(input).await;
        assert!(result.is_err(), "Expected error for invalid input '{}', got: {:?}", input, result);
    }
}
```

## Implementation skeleton

```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Selector {
    pub key: String,
    pub operator: Operator,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operator {
    Equals,
    NotEquals,
    In,
    NotIn,
    Exists,
    DoesNotExist,
}

pub async fn parse(input: &str) -> Result<Vec<Selector>, String> {
    todo!("Tier 2")
}
```
