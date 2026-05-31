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
    finalize_kind, propagate_ingress_status, reconcile_domain_claim, reconcile_kind, resolve_ref,
    ClusterDomainClaim, DomainClaimRegistry, DomainMapping, NetworkConfig, ResolvedUri,
    INGRESS_CLASS_ANNOTATION,
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

// ── Cycle 2: resolveRef — KReference → service DNS resolution ───────────────

const CLUSTER_DOMAIN: &str = "cluster.local";

fn uri(host: &str, path: &str) -> ResolvedUri {
    ResolvedUri {
        host: host.to_string(),
        path: path.to_string(),
        scheme: "http".to_string(),
    }
}

#[test]
fn resolve_ref_extracts_backend_service_from_cluster_dns() {
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.svc.cluster.local", "");
    let (host, backend) = resolve_ref(&mut m, &resolved, CLUSTER_DOMAIN).expect("should resolve");
    assert_eq!(host, "myapp.team-a.svc.cluster.local");
    assert_eq!(backend, "myapp", "backend service is the name component");
    assert!(matches!(
        m.status.conditions.get("ReferenceResolved"),
        Some(cave_knative::broker_controller::ConditionState::True)
    ));
}

#[test]
fn resolve_ref_rejects_target_with_a_path() {
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.svc.cluster.local", "/sub");
    let r = resolve_ref(&mut m, &resolved, CLUSTER_DOMAIN);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("contains a path"));
    assert!(matches!(
        m.status.conditions.get("ReferenceResolved"),
        Some(cave_knative::broker_controller::ConditionState::False(_))
    ));
}

#[test]
fn resolve_ref_treats_bare_trailing_slash_as_no_path() {
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.svc.cluster.local", "/");
    let r = resolve_ref(&mut m, &resolved, CLUSTER_DOMAIN);
    assert!(r.is_ok(), "a lone trailing slash is not a path: {r:?}");
}

#[test]
fn resolve_ref_rejects_non_service_suffix() {
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.example.org", "");
    let r = resolve_ref(&mut m, &resolved, CLUSTER_DOMAIN);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("must be of the form"));
}

#[test]
fn resolve_ref_rejects_host_with_extra_labels() {
    // {name}.{namespace} must be exactly two labels before .svc.<domain>.
    let mut m = dm("example.com", "team-a");
    let resolved = uri("a.b.team-a.svc.cluster.local", "");
    let r = resolve_ref(&mut m, &resolved, CLUSTER_DOMAIN);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("must be of the form"));
}

#[test]
fn resolve_ref_rejects_cross_namespace_target() {
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-b.svc.cluster.local", "");
    let r = resolve_ref(&mut m, &resolved, CLUSTER_DOMAIN);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("same namespace"));
}

// ── Cycle 3: ReconcileKind state machine + Ingress projection + Ready ────────

fn net_cfg() -> NetworkConfig {
    NetworkConfig {
        default_external_scheme: "http".to_string(),
        cluster_domain: CLUSTER_DOMAIN.to_string(),
        autocreate_cluster_domain_claims: true,
        default_ingress_class: "istio.ingress.networking.knative.dev".to_string(),
    }
}

#[test]
fn reconcile_kind_sets_url_and_address_to_domain_name() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.svc.cluster.local", "");
    let ing = reconcile_kind(&mut m, &resolved, &mut reg, &net_cfg()).expect("ok");
    assert_eq!(m.status.url.as_deref(), Some("http://example.com"));
    assert_eq!(m.status.address.as_deref(), Some("http://example.com"));
    assert_eq!(ing.host, "myapp.team-a.svc.cluster.local");
    assert_eq!(ing.backend_service, "myapp");
    assert_eq!(ing.namespace, "team-a");
    assert_eq!(ing.ingress_class, "istio.ingress.networking.knative.dev");
}

#[test]
fn reconcile_kind_honors_ingress_class_annotation_over_default() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    m.metadata
        .annotations
        .insert(INGRESS_CLASS_ANNOTATION.to_string(), "contour.ingress".to_string());
    let resolved = uri("myapp.team-a.svc.cluster.local", "");
    let ing = reconcile_kind(&mut m, &resolved, &mut reg, &net_cfg()).expect("ok");
    assert_eq!(ing.ingress_class, "contour.ingress");
}

#[test]
fn reconcile_kind_leaves_ingress_unconfigured_until_propagated() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.svc.cluster.local", "");
    reconcile_kind(&mut m, &resolved, &mut reg, &net_cfg()).expect("ok");
    // Claim + reference resolved + cert satisfied, but ingress not yet ready.
    assert!(!m.status.is_ready(), "must not be Ready before ingress propagates");
    assert!(matches!(
        m.status.conditions.get("IngressReady"),
        Some(ConditionState::Unknown)
    ));
}

#[test]
fn reconcile_kind_then_ingress_ready_makes_domain_mapping_ready() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    let resolved = uri("myapp.team-a.svc.cluster.local", "");
    reconcile_kind(&mut m, &resolved, &mut reg, &net_cfg()).expect("ok");
    propagate_ingress_status(&mut m, true, true);
    assert!(m.status.is_ready(), "all gating conditions True => Ready");
    assert!(matches!(
        m.status.conditions.get("IngressReady"),
        Some(ConditionState::True)
    ));
}

#[test]
fn propagate_ingress_status_marks_not_configured_on_generation_skew() {
    let mut m = dm("example.com", "team-a");
    // Even if the underlying ingress reports ready, a generation mismatch
    // means the observed status is stale — defensively not-configured.
    propagate_ingress_status(&mut m, true, false);
    assert!(matches!(
        m.status.conditions.get("IngressReady"),
        Some(ConditionState::Unknown)
    ));
}

#[test]
fn propagate_ingress_status_marks_not_ready_when_ingress_failed() {
    let mut m = dm("example.com", "team-a");
    propagate_ingress_status(&mut m, false, true);
    assert!(matches!(
        m.status.conditions.get("IngressReady"),
        Some(ConditionState::False(_))
    ));
    assert!(!m.status.is_ready());
}

#[test]
fn reconcile_kind_fails_closed_on_cross_namespace_claim() {
    let mut reg = DomainClaimRegistry::default();
    reg.create("example.com", "team-a");
    let mut m = dm("example.com", "team-b");
    let resolved = uri("myapp.team-b.svc.cluster.local", "");
    let r = reconcile_kind(&mut m, &resolved, &mut reg, &net_cfg());
    assert!(r.is_err(), "cross-ns claim must abort reconcile");
    // URL/Address are still published, but DomainClaimed is False and not Ready.
    assert_eq!(m.status.url.as_deref(), Some("http://example.com"));
    assert!(matches!(
        m.status.conditions.get("DomainClaimed"),
        Some(ConditionState::False(_))
    ));
    assert!(!m.status.is_ready());
}

#[test]
fn reconcile_kind_byo_tls_secret_marks_certificate_satisfied() {
    let mut reg = DomainClaimRegistry::default();
    let mut m = dm("example.com", "team-a");
    m.spec.tls_secret = Some("my-cert".to_string());
    let resolved = uri("myapp.team-a.svc.cluster.local", "");
    let ing = reconcile_kind(&mut m, &resolved, &mut reg, &net_cfg()).expect("ok");
    assert!(ing.tls, "BYO secret => ingress carries TLS");
    assert!(matches!(
        m.status.conditions.get("CertificateProvisioned"),
        Some(ConditionState::True)
    ));
}
