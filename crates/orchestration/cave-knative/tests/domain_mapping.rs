// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DomainMapping reconciler port — strict-TDD integration tests.
//!
//! upstream: knative/serving pkg/reconciler/domainmapping/reconciler.go
//! (knative-v1.22.0). Mirrors the in-process control-plane logic:
//! ClusterDomainClaim ownership, KReference resolution, status state
//! machine, and Ingress projection. The DNS record + TLS issuance pieces
//! delegate to cave-dns / cave-certs (the latter via src/cert_bridge.rs).

use cave_knative::broker_controller::ConditionState;
use cave_knative::domain_mapping::{
    reconcile_domain_claim, finalize_kind, ClusterDomainClaim, DomainClaimRegistry, DomainMapping,
};

fn dm(name: &str, namespace: &str) -> DomainMapping {
    let mut m = DomainMapping::default();
    m.metadata.name = name.to_string();
    m.metadata.namespace = namespace.to_string();
    m
}

// ── Cycle 1: ClusterDomainClaim ownership + lifecycle ───────────────────────

#[test]
fn claim_autocreated_when_absent_and_autocreate_enabled() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    let r = reconcile_domain_claim(&mut m, &mut reg, true);
    assert!(r.is_ok(), "autocreate should succeed: {r:?}");
    // A claim now exists, owned by team-a.
    assert_eq!(
        reg.get("example.com"),
        Some(&ClusterDomainClaim {
            domain: "example.com".to_string(),
            namespace: "team-a".to_string()
        })
    );
    assert_eq!(
        m.status.conditions.get("DomainClaimed"),
        Some(&ConditionState::True)
    );
}

#[test]
fn claim_rejected_when_absent_and_autocreate_disabled() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    let r = reconcile_domain_claim(&mut m, &mut reg, false);
    assert!(r.is_err(), "no autocreate => must fail");
    assert!(reg.get("example.com").is_none(), "no claim must be created");
    assert!(matches!(
        m.status.conditions.get("DomainClaimed"),
        Some(ConditionState::False(_))
    ));
}

#[test]
fn claim_owned_by_same_namespace_is_accepted() {
    let mut reg = DomainClaimRegistry::default();
    reg.create("example.com", "team-a");
    let mut m = dm("example.com", "team-a");
    let r = reconcile_domain_claim(&mut m, &mut reg, false);
    assert!(r.is_ok(), "same-ns owner should pass even without autocreate");
    assert_eq!(
        m.status.conditions.get("DomainClaimed"),
        Some(&ConditionState::True)
    );
}

#[test]
fn claim_owned_by_other_namespace_is_rejected() {
    let mut reg = DomainClaimRegistry::default();
    reg.create("example.com", "team-a");
    let mut m = dm("example.com", "team-b");
    let r = reconcile_domain_claim(&mut m, &mut reg, true);
    assert!(r.is_err(), "cross-ns collision must be rejected");
    let msg = r.unwrap_err();
    assert!(
        msg.contains("does not own") && msg.contains("team-b"),
        "message should name the losing namespace: {msg}"
    );
    assert!(matches!(
        m.status.conditions.get("DomainClaimed"),
        Some(ConditionState::False(_))
    ));
    // The original owner's claim is untouched.
    assert_eq!(reg.get("example.com").unwrap().namespace, "team-a");
}

#[test]
fn finalize_deletes_owned_claim_when_autocreate_enabled() {
    let mut reg = DomainClaimRegistry::default();
    reg.create("example.com", "team-a");
    let mut m = dm("example.com", "team-a");
    finalize_kind(&mut m, &mut reg, true);
    assert!(reg.get("example.com").is_none(), "owned claim should be cleaned up");
}

#[test]
fn finalize_leaves_claim_when_autocreate_disabled() {
    // When autocreate is off, the operator owns the claim lifecycle; finalize
    // must not delete it.
    let mut reg = DomainClaimRegistry::default();
    reg.create("example.com", "team-a");
    let mut m = dm("example.com", "team-a");
    finalize_kind(&mut m, &mut reg, false);
    assert!(reg.get("example.com").is_some(), "claim must survive finalize");
}

#[test]
fn finalize_does_not_delete_other_namespaces_claim() {
    let mut reg = DomainClaimRegistry::default();
    reg.create("example.com", "team-a");
    let mut m = dm("example.com", "team-b");
    finalize_kind(&mut m, &mut reg, true);
    assert!(
        reg.get("example.com").is_some(),
        "must never delete a claim owned by another namespace"
    );
}
