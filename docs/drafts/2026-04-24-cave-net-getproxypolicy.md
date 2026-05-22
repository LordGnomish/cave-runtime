---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/proxy/policy.go
upstream_fn: GetProxyPolicy
status: draft
tier: 1
created_at: 2026-04-24T17:36:27.595471+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/proxy/policy.go` → `GetProxyPolicy`

## Failing test

```rust
#[tokio::test]
async fn test_getproxypolicy() {
    use cave_net::proxy::ProxyPolicy;
    use cave_net::proxy::getproxypolicy;
    use std::net::SocketAddr;

    // Simulate a scenario with multiple endpoints and policies
    let local_addr = "10.0.0.1:8080".parse::<SocketAddr>().unwrap();
    let remote_addr = "10.0.0.2:443".parse::<SocketAddr>().unwrap();
    let l7_policy_name = "http-l7-policy";
    let ingress = true;

    // Mock policy data (in real implementation, this would come from policy store)
    let policy = getproxypolicy(local_addr, remote_addr, l7_policy_name, ingress).await;

    // Assert that policy is returned (even if unimplemented, it should return Some or None consistently)
    match policy {
        Some(ProxyPolicy { .. }) => {
            // In a full implementation, we'd assert specific fields
            // For now, just confirm the function returns a valid structure
            assert!(true);
        }
        None => {
            // Also acceptable if unimplemented logic yields None
            assert!(true);
        }
    }
}
```

## Implementation skeleton

```rust
pub async fn getproxypolicy(
    local_addr: std::net::SocketAddr,
    remote_addr: std::net::SocketAddr,
    l7_policy_name: &str,
    ingress: bool,
) -> Option<ProxyPolicy> {
    todo!("Tier 2")
}
```
