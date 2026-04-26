---
crate: cave-kernel
upstream_repo: kubernetes/kubernetes
upstream_file: staging/src/k8s.io/apimachinery/pkg/api/resource/quantity.go
upstream_fn: ParseQuantity
status: draft
tier: 1
created_at: 2026-04-24T16:30:29.431828+00:00
---

## Upstream reference

`kubernetes/kubernetes` → `staging/src/k8s.io/apimachinery/pkg/api/resource/quantity.go` → `ParseQuantity`

## Failing test

```rust
#[tokio::test]
async fn test_parsequantity() {
    use cave_kernel::parsequantity;
    use std::str::FromStr;

    // Test basic integer parsing
    let q = parsequantity("100").unwrap();
    assert_eq!(q.to_string(), "100");

    // Test parsing with binary SI suffixes
    let q = parsequantity("1Ki").unwrap();
    assert_eq!(q.to_string(), "1024");

    let q = parsequantity("1Mi").unwrap();
    assert_eq!(q.to_string(), "1048576");

    let q = parsequantity("1Gi").unwrap();
    assert_eq!(q.to_string(), "1073741824");

    // Test parsing with decimal SI suffixes
    let q = parsequantity("1k").unwrap();
    assert_eq!(q.to_string(), "1000");

    let q = parsequantity("1M").unwrap();
    assert_eq!(q.to_string(), "1000000");

    let q = parsequantity("1G").unwrap();
    assert_eq!(q.to_string(), "1000000000");

    // Test negative values
    let q = parsequantity("-1.5Ki").unwrap();
    assert_eq!(q.to_string(), "-1536");

    // Test fractional values
    let q = parsequantity("0.5").unwrap();
    assert_eq!(q.to_string(), "1/2");

    // Test zero
    let q = parsequantity("0").unwrap();
    assert_eq!(q.to_string(), "0");

    // Test invalid input
    let res = parsequantity("abc");
    assert!(res.is_err());

    let res = parsequantity("1x");
    assert!(res.is_err());
}
```

## Implementation skeleton

```rust
pub fn parsequantity(s: &str) -> Result<Quantity, &'static str> {
    todo!("Tier 2")
}
```
