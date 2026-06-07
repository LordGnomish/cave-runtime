// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Headline acceptance test: CiliumNetworkPolicy reconcile (RED→GREEN).
//!
//! Builds a CNP that selects `app=web`, allows ingress only from
//! `app=client` on TCP/80 with an HTTP `GET /api` L7 rule, then reconciles
//! it against a set of known security identities and asserts the resulting
//! policy-map: ingress goes default-deny, the client identity is allowed on
//! 80/TCP with the L7 rule attached, and an unrelated identity is denied.
//! Egress, unselected by any rule, stays default-allow.

use cave_cilium::policy::{
    CiliumNetworkPolicy, Direction, EndpointSelector, Entity, HttpRule, IdentityAllocator,
    IngressRule, L4Port, L7Rules, Labels, Peer, PortRule, Protocol, ReservedIdentity,
    ResolvedIdentity, Rule,
};

fn web_policy() -> CiliumNetworkPolicy {
    let ingress = IngressRule {
        from_endpoints: vec![EndpointSelector::new(&[("app", "client")])],
        from_cidr: vec![],
        from_entities: vec![],
        to_ports: vec![PortRule {
            ports: vec![L4Port {
                port: 80,
                protocol: Protocol::Tcp,
            }],
            l7: Some(L7Rules {
                http: vec![HttpRule {
                    method: Some("GET".into()),
                    path: Some("/api".into()),
                    headers: vec![],
                }],
            }),
        }],
    };
    CiliumNetworkPolicy {
        name: "web-allow-client".into(),
        namespace: "default".into(),
        specs: vec![Rule {
            endpoint_selector: EndpointSelector::new(&[("app", "web")]),
            ingress: vec![ingress],
            egress: vec![],
        }],
    }
}

#[test]
fn reconcile_default_deny_allows_only_matching_peer_with_l7() {
    let mut repo = cave_cilium::policy::PolicyRepository::default();
    repo.add_policy(&web_policy());

    // Identity universe.
    let mut idents = IdentityAllocator::new();
    let web = idents.allocate(&Labels::new(&[("app", "web")]));
    let client = idents.allocate(&Labels::new(&[("app", "client")]));
    let evil = idents.allocate(&Labels::new(&[("app", "evil")]));

    let peers = vec![
        ResolvedIdentity::new(web, Labels::new(&[("app", "web")])),
        ResolvedIdentity::new(client, Labels::new(&[("app", "client")])),
        ResolvedIdentity::new(evil, Labels::new(&[("app", "evil")])),
    ];

    let ingress = repo.resolve(&Labels::new(&[("app", "web")]), Direction::Ingress, &peers);

    // The web endpoint IS selected by a rule with ingress → default-deny.
    assert!(ingress.default_deny, "selected endpoint enters default-deny");

    // Exactly one allow entry: the client identity on 80/TCP with L7.
    assert_eq!(ingress.entries.len(), 1, "only the client peer is allowed");
    let e = &ingress.entries[0];
    assert_eq!(e.peer, Peer::Identity(client));
    assert_eq!(e.port, Some(80));
    assert_eq!(e.protocol, Protocol::Tcp);
    let l7 = e.l7.as_ref().expect("L7 rule attached");
    assert_eq!(l7.http[0].method.as_deref(), Some("GET"));
    assert_eq!(l7.http[0].path.as_deref(), Some("/api"));

    // The evil identity is not in the allow set.
    assert!(
        !ingress.entries.iter().any(|e| e.peer == Peer::Identity(evil)),
        "evil identity must be denied"
    );

    // Egress is unselected by any rule → default-allow (no restriction).
    let egress = repo.resolve(&Labels::new(&[("app", "web")]), Direction::Egress, &peers);
    assert!(!egress.default_deny, "egress unrestricted");
    assert!(egress.entries.is_empty());
}

#[test]
fn unselected_endpoint_has_no_policy() {
    let mut repo = cave_cilium::policy::PolicyRepository::default();
    repo.add_policy(&web_policy());
    // An endpoint nothing selects: both directions default-allow.
    let r = repo.resolve(&Labels::new(&[("app", "db")]), Direction::Ingress, &[]);
    assert!(!r.default_deny);
    assert!(r.entries.is_empty());
}

#[test]
fn cidr_and_entity_rules_resolve_to_non_identity_peers() {
    let mut repo = cave_cilium::policy::PolicyRepository::default();
    repo.add_policy(&CiliumNetworkPolicy {
        name: "egress-world".into(),
        namespace: "default".into(),
        specs: vec![Rule {
            endpoint_selector: EndpointSelector::new(&[("app", "web")]),
            ingress: vec![],
            egress: vec![cave_cilium::policy::EgressRule {
                to_endpoints: vec![],
                to_cidr: vec!["1.1.1.0/24".parse().unwrap()],
                to_entities: vec![Entity::World],
                to_ports: vec![PortRule {
                    ports: vec![L4Port {
                        port: 443,
                        protocol: Protocol::Tcp,
                    }],
                    l7: None,
                }],
            }],
        }],
    });

    let r = repo.resolve(&Labels::new(&[("app", "web")]), Direction::Egress, &[]);
    assert!(r.default_deny, "egress now restricted");
    assert!(r.entries.iter().any(|e| e.peer == Peer::Cidr("1.1.1.0/24".parse().unwrap())));
    assert!(r
        .entries
        .iter()
        .any(|e| e.peer == Peer::Entity(Entity::World)));
    assert!(r.entries.iter().all(|e| e.port == Some(443)));
}

#[test]
fn identities_are_stable_and_reserved_match_cilium() {
    let mut a = IdentityAllocator::new();
    let one = a.allocate(&Labels::new(&[("app", "web")]));
    let two = a.allocate(&Labels::new(&[("app", "web")]));
    assert_eq!(one, two, "same label-set → same identity");
    assert!(one >= 256, "allocated identities start at 256, got {}", one);
    assert_ne!(one, a.allocate(&Labels::new(&[("app", "other")])));

    // Reserved identity ordinals must match cilium's numericidentity.go.
    assert_eq!(ReservedIdentity::Host as u32, 1);
    assert_eq!(ReservedIdentity::World as u32, 2);
    assert_eq!(ReservedIdentity::RemoteNode as u32, 6);
    assert_eq!(ReservedIdentity::KubeApiServer as u32, 7);
}

#[test]
fn endpoint_selector_is_subset_match() {
    let sel = EndpointSelector::new(&[("app", "web")]);
    assert!(sel.matches(&Labels::new(&[("app", "web"), ("tier", "frontend")])));
    assert!(!sel.matches(&Labels::new(&[("app", "db")])));
    // Empty selector matches everything (cilium "{}" selects all).
    assert!(EndpointSelector::new(&[]).matches(&Labels::new(&[("x", "y")])));
}
