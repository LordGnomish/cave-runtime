---
crate: cave-vault
upstream_repo: openbao/openbao
upstream_file: sdk/framework/path.go
upstream_fn: validate
status: draft
tier: 1
created_at: 2026-04-24T16:49:56.320296+00:00
---

## Upstream reference

`openbao/openbao` → `sdk/framework/path.go` → `validate`

## Failing test

```rust
#[tokio::test]
async fn test_validate_path_patterns() {
    use cave_vault::validate;
    use std::collections::HashMap;

    // Test case 1: exact match
    let path = "secret/data/foo";
    let patterns = vec!["secret/data/foo"];
    let result = validate(path, &patterns).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "secret/data/foo");

    // Test case 2: wildcard match at end
    let path = "secret/data/bar/baz";
    let patterns = vec!["secret/data/*"];
    let result = validate(path, &patterns).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "secret/data/*");

    // Test case 3: multiple wildcards
    let path = "secret/data/foo/bar/baz";
    let patterns = vec!["secret/data/*"];
    let result = validate(path, &patterns).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "secret/data/*");

    // Test case 4: no match
    let path = "secret/data/foo";
    let patterns = vec!["sys/*"];
    let result = validate(path, &patterns).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no matching path pattern"));

    // Test case 5: empty patterns
    let path = "secret/data/foo";
    let patterns: Vec<&str> = vec![];
    let result = validate(path, &patterns).await;
    assert!(result.is_err());

    // Test case 6: path with parameters (e.g., "auth/cert/login/:cert_name")
    let path = "auth/cert/login/my-cert";
    let patterns = vec!["auth/cert/login/:cert_name"];
    let result = validate(path, &patterns).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "auth/cert/login/:cert_name");

    // Test case 7: parameter mismatch
    let path = "auth/cert/login";
    let patterns = vec!["auth/cert/login/:cert_name"];
    let result = validate(path, &patterns).await;
    assert!(result.is_err());
}
```

## Implementation skeleton

```rust
use std::collections::HashMap;

pub async fn validate(path: &str, patterns: &[&str]) -> Result<String, String> {
    todo!("Tier 2")
}
```
