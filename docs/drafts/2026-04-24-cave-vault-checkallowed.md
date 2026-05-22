---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: vault/policy.go
upstream_fn: CheckAllowed
status: draft
tier: 1
created_at: 2026-04-24T16:49:28.829430+00:00
---

## Upstream reference

`openbao/openbao` → `vault/policy.go` → `CheckAllowed`

## Failing test

```rust
#[tokio::test]
async fn test_checkallowed() {
    use cave_vault::{PolicySet, Policy, CheckAllowedInput, CheckAllowedOutput, Permission};
    use std::collections::HashSet;

    // Create a policy set with two policies
    let mut policies = PolicySet::new();
    
    let mut policy1 = Policy::new("admin");
    policy1.add_permission(Permission::Read("secret/data/*".to_string()));
    policy1.add_permission(Permission::Write("secret/data/*".to_string()));
    policies.insert("admin".to_string(), policy1);

    let mut policy2 = Policy::new("reader");
    policy2.add_permission(Permission::Read("secret/data/public/*".to_string()));
    policies.insert("reader".to_string(), policy2);

    // Test case 1: admin with full access
    let input1 = CheckAllowedInput {
        policies: vec!["admin".to_string()],
        path: "secret/data/sensitive".to_string(),
        operation: Permission::Read,
    };
    let result1 = checkallowed(input1, &policies).await;
    assert!(result1.allowed);
    assert!(result1.errors.is_empty());

    // Test case 2: reader with partial access (should allow public path)
    let input2 = CheckAllowedInput {
        policies: vec!["reader".to_string()],
        path: "secret/data/public/info".to_string(),
        operation: Permission::Read,
    };
    let result2 = checkallowed(input2, &policies).await;
    assert!(result2.allowed);
    assert!(result2.errors.is_empty());

    // Test case 3: reader trying to access non-public path (should deny)
    let input3 = CheckAllowedInput {
        policies: vec!["reader".to_string()],
        path: "secret/data/sensitive".to_string(),
        operation: Permission::Read,
    };
    let result3 = checkallowed(input3, &policies).await;
    assert!(!result3.allowed);
    assert!(result3.errors.contains(&"permission denied".to_string()));

    // Test case 4: multiple policies (admin + reader) with admin granting access
    let input4 = CheckAllowedInput {
        policies: vec!["reader".to_string(), "admin".to_string()],
        path: "secret/data/sensitive".to_string(),
        operation: Permission::Write,
    };
    let result4 = checkallowed(input4, &policies).await;
    assert!(result4.allowed);
    assert!(result4.errors.is_empty());

    // Test case 5: no matching policies
    let input5 = CheckAllowedInput {
        policies: vec!["unknown".to_string()],
        path: "secret/data/public/test".to_string(),
        operation: Permission::Read,
    };
    let result5 = checkallowed(input5, &policies).await;
    assert!(!result5.allowed);
    assert!(result5.errors.contains(&"no matching policy found".to_string()));
}
```

## Implementation skeleton

```rust
pub async fn checkallowed(
    input: CheckAllowedInput,
    policies: &PolicySet,
) -> CheckAllowedOutput {
    todo!("Tier 2")
}
```
