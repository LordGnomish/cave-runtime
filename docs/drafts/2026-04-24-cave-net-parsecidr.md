---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/cidr/cidr.go
upstream_fn: ParseCIDR
status: draft
tier: 1
created_at: 2026-04-24T16:41:16.951339+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/cidr/cidr.go` → `ParseCIDR`

## Failing test

```rust
#[tokio::test]
async fn test_parsecidr() {
    use cave_net::parsecidr;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use std::str::FromStr;

    // Valid IPv4 CIDR
    let (ip, prefix_len) = parsecidr("192.168.1.0/24").await.unwrap();
    assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0)));
    assert_eq!(prefix_len, 24);

    // Valid IPv6 CIDR
    let (ip, prefix_len) = parsecidr("2001:db8::/32").await.unwrap();
    assert_eq!(ip, IpAddr::V6(Ipv6Addr::from_str("2001:db8::").unwrap()));
    assert_eq!(prefix_len, 32);

    // Valid IPv4 without prefix (assume /32)
    let (ip, prefix_len) = parsecidr("10.0.0.1").await.unwrap();
    assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
    assert_eq!(prefix_len, 32);

    // Valid IPv6 without prefix (assume /128)
    let (ip, prefix_len) = parsecidr("::1").await.unwrap();
    assert_eq!(ip, IpAddr::V6(Ipv6Addr::LOCALHOST));
    assert_eq!(prefix_len, 128);

    // Invalid CIDR (malformed)
    let err = parsecidr("192.168.1.0/33").await.unwrap_err();
    assert!(err.to_string().contains("prefix length"));

    // Invalid IP
    let err = parsecidr("256.1.1.1/24").await.unwrap_err();
    assert!(err.to_string().contains("invalid IP"));

    // Empty string
    let err = parsecidr("").await.unwrap_err();
    assert!(err.to_string().contains("empty"));
}
```

## Implementation skeleton

```rust
pub async fn parsecidr(cidr: &str) -> Result<(IpAddr, u8), Box<dyn std::error::Error + Send + Sync>> {
    todo!("Tier 2")
}
```
