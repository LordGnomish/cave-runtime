---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/labels/labels.go
upstream_fn: ParseLabel
status: draft
tier: 1
created_at: 2026-04-24T16:42:40.239606+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/labels/labels.go` → `ParseLabel`

## Failing test

```rust
#[tokio::test]
async fn test_parselabel() {
    use cave_net::parselabel;

    // Valid label with key-value pair
    let label = "k1=v1";
    let (key, value) = parselabel(label).await.unwrap();
    assert_eq!(key, "k1");
    assert_eq!(value, "v1");

    // Valid label with key only (no value)
    let label = "k2";
    let (key, value) = parselabel(label).await.unwrap();
    assert_eq!(key, "k2");
    assert_eq!(value, "");

    // Label with empty key should fail
    let label = "=v3";
    assert!(parselabel(label).await.is_err());

    // Label with empty key and value should fail
    let label = "=";
    assert!(parselabel(label).await.is_err());

    // Label with empty string should fail
    let label = "";
    assert!(parselabel(label).await.is_err());

    // Label with multiple '=' should parse first as key-value, rest as value
    let label = "k4=v4=v5";
    let (key, value) = parselabel(label).await.unwrap();
    assert_eq!(key, "k4");
    assert_eq!(value, "v4=v5");
}
```

## Implementation skeleton

```rust
pub async fn parselabel(label: &str) -> Result<(String, String), &'static str> {
    todo!("Tier 2")
}
```
