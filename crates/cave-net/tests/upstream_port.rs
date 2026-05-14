//! Line-by-line ports of upstream Cilium tests, cross-referenced
//! from `parity.manifest.toml`'s `[[upstream_test]]` block.
//!
//! Upstream: cilium/cilium @ v1.19.3
//!   * pkg/identity/cache/local_test.go
//!   * pkg/identity/numericidentity_test.go
//!   * pkg/endpoint/endpoint_test.go
//!   * pkg/policy/api/selector_test.go
//!   * pkg/policy/distillery_test.go
//!   * pkg/policy/cidr_test.go
//!   * pkg/policy/l4_test.go
//!
//! Cilium's bpf/* and datapath/loader tests are deliberately skipped
//! — they need a running kernel + clang toolchain. Userspace tests
//! around identity, selector, policy distillation, and endpoint state
//! map cleanly to the cave-net public API.

use cave_net::cilium::endpoint::{BpfProgram, Endpoint, EndpointError, EndpointManager, EndpointState};
use cave_net::cilium::identity::{
    LabelSet, LocalIdentityCache, MIN_LOCAL_IDENTITY, reserved_identity_for, ID_HOST, ID_WORLD,
};
use cave_net::cilium::policy::{
    CidrRule, Direction, EndpointSelector, L4Protocol, MatchExpression, PolicyKey, PolicyMap,
    PortProtocol, SelectorOp, Verdict,
};
use cave_net::cilium::types::TenantId;
use std::collections::HashMap;
use std::net::IpAddr;

fn ls(pairs: &[(&str, &str)]) -> LabelSet {
    LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
}

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).expect("valid tenant fixture")
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/identity/cache/local_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestLocalIdentityCache / `lookupOrCreate_starts_at_min_local_id`.
#[test]
fn upstream_local_identity_cache_first_alloc_is_min_local_id() {
    let mut cache = LocalIdentityCache::new(tenant("acme"));
    let id = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
    assert_eq!(id, MIN_LOCAL_IDENTITY);
}

/// Upstream: TestLocalIdentityCache / `same_labels_same_id`.
/// Idempotent allocation: the same normalised label set always yields
/// the same numeric identity.
#[test]
fn upstream_local_identity_cache_same_labels_same_id() {
    let mut cache = LocalIdentityCache::new(tenant("acme"));
    let id1 = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
    let id2 = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
    assert_eq!(id1, id2);
}

/// Upstream: TestLocalIdentityCache / `labels_order_independent`.
/// `labels.Labels.Sort` in upstream — different input order, same id.
#[test]
fn upstream_local_identity_cache_label_order_does_not_affect_id() {
    let mut cache = LocalIdentityCache::new(tenant("acme"));
    let a = cache
        .lookup_or_allocate(&ls(&[("app", "web"), ("env", "prod")]))
        .unwrap();
    let b = cache
        .lookup_or_allocate(&ls(&[("env", "prod"), ("app", "web")]))
        .unwrap();
    assert_eq!(a, b);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/identity/numericidentity_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestReservedIdentity / `host_label_resolves_to_ID_HOST`.
#[test]
fn upstream_reserved_identity_for_host_resolves_to_id_host() {
    let id = reserved_identity_for(&ls(&[("reserved", "host")]));
    assert_eq!(id, Some(ID_HOST));
}

/// Upstream: TestReservedIdentity / `world_label_resolves_to_ID_WORLD`.
#[test]
fn upstream_reserved_identity_for_world_resolves_to_id_world() {
    let id = reserved_identity_for(&ls(&[("reserved", "world")]));
    assert_eq!(id, Some(ID_WORLD));
}

/// Upstream: TestReservedIdentity / `unknown_reserved_label_returns_none`.
/// Upstream: `getReservedID` returns 0 for unknown.
#[test]
fn upstream_reserved_identity_for_unknown_reserved_label_is_none() {
    let id = reserved_identity_for(&ls(&[("reserved", "made-up-name")]));
    assert_eq!(id, None);
}

/// Upstream: TestLocalIdentityCache / `reserved_label_does_not_consume_slot`.
/// Allocating a reserved-label set MUST return the reserved ID, NOT
/// a local slot. The first non-reserved allocation should still come
/// back as MIN_LOCAL_IDENTITY.
#[test]
fn upstream_reserved_label_does_not_consume_local_slot() {
    let mut cache = LocalIdentityCache::new(tenant("acme"));
    let host = cache.lookup_or_allocate(&ls(&[("reserved", "host")])).unwrap();
    assert_eq!(host, ID_HOST);
    let real = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
    assert_eq!(real, MIN_LOCAL_IDENTITY);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/endpoint/endpoint_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestEndpoint / `state_transitions_creating_to_ready`.
/// Upstream lifecycle: Creating → WaitingForIdentity → Ready.
#[test]
fn upstream_endpoint_state_transitions_creating_to_ready() {
    let mut mgr = EndpointManager::new();
    let ip: IpAddr = "10.0.0.1".parse().unwrap();
    let id = mgr.create(tenant("acme"), "web-1", "default", ip);
    assert_eq!(mgr.lookup(id).unwrap().state, EndpointState::Creating);
    mgr.transition(id, EndpointState::WaitingForIdentity).unwrap();
    mgr.transition(id, EndpointState::Ready).unwrap();
    assert_eq!(mgr.lookup(id).unwrap().state, EndpointState::Ready);
}

/// Upstream: TestEndpoint / `bad_state_transition_rejected`.
/// Upstream `SetState` refuses Creating → Ready directly.
#[test]
fn upstream_endpoint_bad_state_transition_rejected() {
    let mut mgr = EndpointManager::new();
    let id = mgr.create(
        tenant("acme"),
        "w",
        "default",
        "10.0.0.2".parse().unwrap(),
    );
    let err = mgr
        .transition(id, EndpointState::Ready)
        .unwrap_err();
    assert!(
        matches!(err, EndpointError::BadTransition { .. }),
        "expected BadTransition, got {err:?}"
    );
}

/// Upstream: TestEndpoint / `lookup_by_pod_ip_returns_endpoint`.
#[test]
fn upstream_endpoint_lookup_by_pod_ip_returns_endpoint() {
    let mut mgr = EndpointManager::new();
    let ip: IpAddr = "10.0.0.3".parse().unwrap();
    let id = mgr.create(tenant("acme"), "w", "default", ip);
    let fetched = mgr.lookup_by_pod_ip(ip).unwrap();
    assert_eq!(fetched.id, id);
}

/// Upstream: TestEndpoint / `insert_duplicate_id_errors`.
#[test]
fn upstream_endpoint_manager_insert_duplicate_id_errors() {
    let mut mgr = EndpointManager::new();
    let ip: IpAddr = "10.0.0.4".parse().unwrap();
    let ep = Endpoint::new_creating(42, tenant("acme"), "w", "default", ip);
    mgr.insert(ep.clone()).unwrap();
    let err = mgr.insert(ep).unwrap_err();
    assert!(matches!(err, EndpointError::DuplicateId(42)));
}

/// Upstream: TestProgramChain / `set_program_chain_persists_into_endpoint`.
/// Models the per-endpoint BPF tail-call chain that bpf_lxc.c compiles in.
#[test]
fn upstream_endpoint_set_program_chain_persists() {
    let mut mgr = EndpointManager::new();
    let id = mgr.create(
        tenant("acme"),
        "w",
        "default",
        "10.0.0.5".parse().unwrap(),
    );
    let chain = vec![
        BpfProgram::FromContainer,
        BpfProgram::Conntrack,
        BpfProgram::Policy,
        BpfProgram::ToLxc,
    ];
    mgr.set_program_chain(id, chain.clone()).unwrap();
    assert_eq!(mgr.lookup(id).unwrap().program_chain, chain);
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/policy/api/selector_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestEndpointSelector / `match_labels_AND_semantics`.
#[test]
fn upstream_endpoint_selector_match_labels_AND_semantics() {
    let mut sel = EndpointSelector::empty();
    sel.match_labels
        .insert("app".into(), "web".into());
    sel.match_labels
        .insert("env".into(), "prod".into());
    assert!(sel.matches(&ls(&[("app", "web"), ("env", "prod")])));
    // Missing one label → no match.
    assert!(!sel.matches(&ls(&[("app", "web")])));
    // Wrong value → no match.
    assert!(!sel.matches(&ls(&[("app", "web"), ("env", "dev")])));
}

/// Upstream: TestEndpointSelector / `In_operator_matches_value_in_set`.
#[test]
fn upstream_endpoint_selector_in_operator_matches_value_in_set() {
    let sel = EndpointSelector {
        match_labels: HashMap::new(),
        match_expressions: vec![MatchExpression {
            key: "env".into(),
            op: SelectorOp::In,
            values: vec!["prod".into(), "staging".into()],
        }],
    };
    assert!(sel.matches(&ls(&[("env", "prod")])));
    assert!(sel.matches(&ls(&[("env", "staging")])));
    assert!(!sel.matches(&ls(&[("env", "dev")])));
    // Missing label → In can never match.
    assert!(!sel.matches(&ls(&[("other", "x")])));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/policy/l4_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestProtoMatch / `ProtoAny_covers_TCP_UDP_SCTP_but_not_ICMP`.
#[test]
fn upstream_l4_protocol_any_covers_tcp_udp_sctp_but_not_icmp() {
    assert!(L4Protocol::Any.covers(L4Protocol::TCP));
    assert!(L4Protocol::Any.covers(L4Protocol::UDP));
    assert!(L4Protocol::Any.covers(L4Protocol::SCTP));
    // Cilium spec: Any does NOT include ICMP.
    assert!(!L4Protocol::Any.covers(L4Protocol::ICMP));
}

/// Upstream: TestPortRule / `port_zero_means_any_port`.
#[test]
fn upstream_l4_port_zero_in_rule_matches_any_wire_port() {
    let rule = PortProtocol::new(0, L4Protocol::TCP);
    assert!(rule.covers(80, L4Protocol::TCP));
    assert!(rule.covers(443, L4Protocol::TCP));
    // Wrong protocol → no match even with port=0.
    assert!(!rule.covers(80, L4Protocol::UDP));
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/policy/cidr_test.go
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestCIDRRule / `contains_inside_cidr`.
#[test]
fn upstream_cidr_rule_contains_address_inside_cidr() {
    let rule = CidrRule::new("10.0.0.0/8");
    assert!(rule.contains("10.5.3.1".parse().unwrap()).unwrap());
    assert!(!rule.contains("192.168.1.1".parse().unwrap()).unwrap());
}

/// Upstream: TestCIDRRule / `except_subblock_is_excluded`.
#[test]
fn upstream_cidr_rule_except_subblock_excluded() {
    let rule = CidrRule::new("10.0.0.0/8").with_except(["10.5.0.0/16"]);
    // Outside the except → still allowed.
    assert!(rule.contains("10.1.1.1".parse().unwrap()).unwrap());
    // Inside the except → excluded.
    assert!(!rule.contains("10.5.0.1".parse().unwrap()).unwrap());
}

// ────────────────────────────────────────────────────────────────────────────
// Upstream: pkg/policy/distillery_test.go (PolicyMap lookup precedence)
// ────────────────────────────────────────────────────────────────────────────

/// Upstream: TestPolicyMap / `default_deny_when_ingress_enforced_and_no_match`.
/// Mirrors `policy_can_access` default-deny fallback.
#[test]
fn upstream_policy_map_default_deny_when_enforced_no_match() {
    let mut map = PolicyMap::new();
    map.ingress_enforced = true;
    let entry = map.lookup(42, 80, L4Protocol::TCP, Direction::Ingress);
    assert_eq!(entry.verdict, Verdict::Deny);
}

/// Upstream: TestPolicyMap / `exact_match_allow_overrides_default_deny`.
#[test]
fn upstream_policy_map_exact_match_allow_overrides_default_deny() {
    let mut map = PolicyMap::new();
    map.ingress_enforced = true;
    let key = PolicyKey {
        peer_identity: 42,
        port: 80,
        protocol: L4Protocol::TCP,
        direction: Direction::Ingress,
    };
    map.allow(key, None);
    let entry = map.lookup(42, 80, L4Protocol::TCP, Direction::Ingress);
    assert_eq!(entry.verdict, Verdict::Allow);
}

/// Upstream: TestPolicyMap / `wildcard_port_zero_covers_specific_port`.
/// `policymap` entry (peer, port=0, exact-proto) matches any port from
/// the same peer over that protocol.
#[test]
fn upstream_policy_map_wildcard_port_zero_covers_specific_port() {
    let mut map = PolicyMap::new();
    map.ingress_enforced = true;
    let wildcard = PolicyKey {
        peer_identity: 42,
        port: 0,
        protocol: L4Protocol::TCP,
        direction: Direction::Ingress,
    };
    map.allow(wildcard, None);
    let entry80 = map.lookup(42, 80, L4Protocol::TCP, Direction::Ingress);
    let entry443 = map.lookup(42, 443, L4Protocol::TCP, Direction::Ingress);
    assert_eq!(entry80.verdict, Verdict::Allow);
    assert_eq!(entry443.verdict, Verdict::Allow);
}
