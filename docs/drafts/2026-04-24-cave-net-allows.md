---
crate: cave-net
upstream_repo: cilium/cilium
upstream_file: pkg/policy/rule.go
upstream_fn: Allows
status: draft
tier: 1
created_at: 2026-04-24T17:36:07.296197+00:00
---

## Upstream reference

`cilium/cilium` → `pkg/policy/rule.go` → `Allows`

## Failing test

```rust
#[tokio::test]
async fn test_allows_policy_decision() {
    use cave_net::policy::{Rule, RuleAction, L4Filter, Protocol, PortRule, EndpointSelector, LabelsSelector};
    use cave_net::model::{Endpoint, Labels};

    // Create a rule that allows HTTP traffic from endpoints with label "app=frontend"
    let rule = Rule {
        endpoint_selector: EndpointSelector::Labels(LabelsSelector {
            labels: vec!["app=frontend".to_string()].into_iter().collect(),
        }),
        l4_filters: vec![
            L4Filter {
                protocol: Protocol::TCP,
                ports: vec![PortRule::PortRange { start: 80, end: 80 }],
            },
            L4Filter {
                protocol: Protocol::TCP,
                ports: vec![PortRule::PortRange { start: 443, end: 443 }],
            },
        ],
        action: RuleAction::Allow,
    };

    // Create an endpoint with matching labels
    let endpoint = Endpoint {
        labels: Labels::from_iter(vec![
            ("app".to_string(), "frontend".to_string()),
            ("tier".to_string(), "web".to_string()),
        ]),
        ..Default::default()
    };

    // Test: should allow HTTP/HTTPS from frontend endpoint
    assert!(cave_net::allows(&rule, &endpoint, 80, Protocol::TCP).await);
    assert!(cave_net::allows(&rule, &endpoint, 443, Protocol::TCP).await);
    assert!(!cave_net::allows(&rule, &endpoint, 8080, Protocol::TCP).await);
    assert!(!cave_net::allows(&rule, &endpoint, 80, Protocol::UDP).await);

    // Test: should deny from non-matching endpoint
    let non_matching_endpoint = Endpoint {
        labels: Labels::from_iter(vec![
            ("app".to_string(), "backend".to_string()),
        ]),
        ..Default::default()
    };
    assert!(!cave_net::allows(&rule, &non_matching_endpoint, 80, Protocol::TCP).await);
}
```

## Implementation skeleton

```rust
pub async fn allows(rule: &Rule, endpoint: &Endpoint, port: u16, protocol: Protocol) -> bool {
    todo!("Tier 2")
}
```
