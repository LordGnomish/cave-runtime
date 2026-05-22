---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: vault/policy.go
upstream_fn: ParseACLPolicy
status: draft
tier: 1
created_at: 2026-04-24T16:48:41.397561+00:00
---

## Upstream reference

`openbao/openbao` → `vault/policy.go` → `ParseACLPolicy`

## Failing test

```rust
#[tokio::test]
async fn test_parseaclpolicy() {
    use cave_vault::parseaclpolicy;
    use std::collections::HashMap;

    // Test case 1: Valid simple policy
    let policy_str = r#"
        path "secret/data/foo" {
            capabilities = ["read", "list"]
        }
        path "secret/data/bar" {
            capabilities = ["create", "update", "delete"]
        }
    "#;
    
    let result = parseaclpolicy(policy_str).await;
    assert!(result.is_ok());
    let policy = result.unwrap();
    assert_eq!(policy.len(), 2);
    assert!(policy.contains_key("secret/data/foo"));
    assert!(policy.contains_key("secret/data/bar"));
    
    let foo_caps = &policy["secret/data/foo"];
    assert!(foo_caps.contains(&"read".to_string()));
    assert!(foo_caps.contains(&"list".to_string()));
    assert_eq!(foo_caps.len(), 2);

    let bar_caps = &policy["secret/data/bar"];
    assert!(bar_caps.contains(&"create".to_string()));
    assert!(bar_caps.contains(&"update".to_string()));
    assert!(bar_caps.contains(&"delete".to_string()));
    assert_eq!(bar_caps.len(), 3);

    // Test case 2: Invalid policy syntax
    let invalid_policy = r#"
        path "secret/data/baz" {
            capabilities = ["read", "invalid_capability"]
        }
    "#;
    
    let result = parseaclpolicy(invalid_policy).await;
    assert!(result.is_err());

    // Test case 3: Empty policy
    let empty_policy = "";
    let result = parseaclpolicy(empty_policy).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());

    // Test case 4: Policy with multiple capabilities on single line
    let multi_cap_policy = r#"
        path "sys/*" {
            capabilities = ["read", "list", "update", "delete", "sudo"]
        }
    "#;
    
    let result = parseaclpolicy(multi_cap_policy).await;
    assert!(result.is_ok());
    let policy = result.unwrap();
    assert_eq!(policy.len(), 1);
    let sys_caps = &policy["sys/*"];
    assert!(sys_caps.contains(&"read".to_string()));
    assert!(sys_caps.contains(&"list".to_string()));
    assert!(sys_caps.contains(&"update".to_string()));
    assert!(sys_caps.contains(&"delete".to_string()));
    assert!(sys_caps.contains(&"sudo".to_string()));
    assert_eq!(sys_caps.len(), 5);
}
```

## Implementation skeleton

```rust
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseACLError {
    #[error("Invalid capability: {0}")]
    InvalidCapability(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

pub type ParseACLResult = Result<HashMap<String, Vec<String>>, ParseACLError>;

pub async fn parseaclpolicy(policy_str: &str) -> Result<HashMap<String, Vec<String>>, ParseACLError> {
    todo!("Tier 2")
}
```
