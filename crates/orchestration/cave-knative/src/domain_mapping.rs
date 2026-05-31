// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! DomainMapping reconciler — map a custom domain onto a Knative target.
//!
//! upstream: knative/serving pkg/reconciler/domainmapping/reconciler.go
//! (knative-v1.22.0), pkg/apis/serving/v1beta1/domainmapping_types.go.
//!
//! A `DomainMapping` binds a fully-qualified domain (its `metadata.name`)
//! to a Knative addressable (`spec.ref`, a KReference to a Service/Route).
//! The reconciler is split into three in-process concerns that this module
//! ports 1:1:
//!
//!   1. ClusterDomainClaim ownership — a cluster-scoped record asserting
//!      that exactly one namespace owns a given domain, so two
//!      DomainMappings can never claim the same hostname (`reconcileDomainClaim`
//!      / `createDomainClaim` / `FinalizeKind`).
//!   2. Reference resolution — resolve `spec.ref` to the standard k8s
//!      service DNS name `{name}.{namespace}.svc.{cluster-domain}`, reject
//!      paths and cross-namespace / non-service targets (`resolveRef`).
//!   3. Status state machine + Ingress projection — drive the
//!      DomainClaimed / ReferenceResolved / IngressReady /
//!      CertificateProvisioned conditions and emit the desired KIngress
//!      (`ReconcileKind` / `MakeIngress` / `PropagateIngressStatus`).
//!
//! The DNS record creation and the TLS certificate issuance themselves are
//! cross-crate: DNS belongs to cave-dns and the certificate flows through
//! [`crate::cert_bridge`] to cert-manager. This module is the Knative
//! control-plane that decides *what* those crates are asked to do.

use crate::broker_controller::ConditionState;
use crate::meta::ObjectMeta;
use std::collections::HashMap;

/// KReference — the object a DomainMapping points at (Service / Route / …).
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct KReference {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    pub namespace: String,
}

#[derive(Default, Debug, Clone)]
pub struct DomainMappingSpec {
    pub ref_: KReference,
    /// Optional bring-your-own TLS secret. When set, the certificate is
    /// considered "provided externally" and Knative issues nothing.
    pub tls_secret: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct DomainMappingStatus {
    pub url: Option<String>,
    pub address: Option<String>,
    pub conditions: HashMap<String, ConditionState>,
    pub observed_generation: i64,
}

/// Condition types that gate `Ready`, in upstream `domainMappingCondSet`.
pub const COND_DOMAIN_CLAIMED: &str = "DomainClaimed";
pub const COND_REFERENCE_RESOLVED: &str = "ReferenceResolved";
pub const COND_INGRESS_READY: &str = "IngressReady";
pub const COND_CERTIFICATE_PROVISIONED: &str = "CertificateProvisioned";

const READY_DEPENDENCIES: [&str; 4] = [
    COND_DOMAIN_CLAIMED,
    COND_REFERENCE_RESOLVED,
    COND_INGRESS_READY,
    COND_CERTIFICATE_PROVISIONED,
];

impl DomainMappingStatus {
    fn set(&mut self, name: &str, state: ConditionState) {
        self.conditions.insert(name.to_string(), state);
    }

    pub fn mark_domain_claimed(&mut self) {
        self.set(COND_DOMAIN_CLAIMED, ConditionState::True);
    }
    pub fn mark_domain_claim_not_owned(&mut self) {
        self.set(
            COND_DOMAIN_CLAIMED,
            ConditionState::False("namespace does not own the cluster domain claim".to_string()),
        );
    }
    pub fn mark_reference_resolved(&mut self) {
        self.set(COND_REFERENCE_RESOLVED, ConditionState::True);
    }
    pub fn mark_reference_not_resolved(&mut self, reason: &str) {
        self.set(
            COND_REFERENCE_RESOLVED,
            ConditionState::False(reason.to_string()),
        );
    }
    pub fn mark_ingress_not_configured(&mut self) {
        self.set(COND_INGRESS_READY, ConditionState::Unknown);
    }
    pub fn mark_ingress_ready(&mut self) {
        self.set(COND_INGRESS_READY, ConditionState::True);
    }
    pub fn mark_ingress_not_ready(&mut self, reason: &str) {
        self.set(COND_INGRESS_READY, ConditionState::False(reason.to_string()));
    }
    /// No TLS work required (no external-domain-TLS, or BYO cert supplied).
    pub fn mark_certificate_not_required(&mut self) {
        self.set(COND_CERTIFICATE_PROVISIONED, ConditionState::True);
    }
    pub fn mark_certificate_ready(&mut self) {
        self.set(COND_CERTIFICATE_PROVISIONED, ConditionState::True);
    }
    pub fn mark_certificate_not_ready(&mut self, reason: &str) {
        self.set(
            COND_CERTIFICATE_PROVISIONED,
            ConditionState::False(reason.to_string()),
        );
    }

    /// Aggregate `Ready` — all gating conditions must be `True`.
    pub fn is_ready(&self) -> bool {
        READY_DEPENDENCIES
            .iter()
            .all(|c| matches!(self.conditions.get(*c), Some(ConditionState::True)))
    }
}

#[derive(Default, Debug, Clone)]
pub struct DomainMapping {
    pub metadata: ObjectMeta,
    pub spec: DomainMappingSpec,
    pub status: DomainMappingStatus,
}

impl DomainMapping {
    pub fn new(tenant_id: &str, domain: &str, namespace: &str) -> Self {
        let mut m = DomainMapping::default();
        m.metadata = ObjectMeta::with_creator(tenant_id);
        m.metadata.name = domain.to_string();
        m.metadata.namespace = namespace.to_string();
        m
    }
}

/// A cluster-scoped record that namespace `namespace` owns domain `domain`.
/// Upstream: networking.internal.knative.dev/v1alpha1 ClusterDomainClaim.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct ClusterDomainClaim {
    pub domain: String,
    pub namespace: String,
}

/// In-process stand-in for the ClusterDomainClaim lister/client. The real
/// reconciler reads/writes these through the networking client; the
/// ownership *decision* it makes is what we port here.
#[derive(Default, Debug, Clone)]
pub struct DomainClaimRegistry {
    claims: HashMap<String, ClusterDomainClaim>,
}

impl DomainClaimRegistry {
    pub fn get(&self, domain: &str) -> Option<&ClusterDomainClaim> {
        self.claims.get(domain)
    }

    pub fn create(&mut self, domain: &str, namespace: &str) {
        self.claims.insert(
            domain.to_string(),
            ClusterDomainClaim {
                domain: domain.to_string(),
                namespace: namespace.to_string(),
            },
        );
    }

    pub fn delete(&mut self, domain: &str) {
        self.claims.remove(domain);
    }
}

/// Port of `reconcileDomainClaim` + `createDomainClaim`.
///
/// Ensures the DomainMapping can create, or already owns, a cluster-wide
/// claim on its domain. Sets the `DomainClaimed` condition. Returns `Err`
/// (with the upstream message shape) when the domain is owned by another
/// namespace, or is unclaimed and autocreate is disabled.
pub fn reconcile_domain_claim(
    dm: &mut DomainMapping,
    registry: &mut DomainClaimRegistry,
    autocreate_cluster_domain_claims: bool,
) -> Result<(), String> {
    let domain = dm.metadata.name.clone();
    let ns = dm.metadata.namespace.clone();

    match registry.get(&domain) {
        None => {
            // createDomainClaim
            if !autocreate_cluster_domain_claims {
                dm.status.mark_domain_claim_not_owned();
                return Err(format!(
                    "no ClusterDomainClaim found for domain {domain:?} (and \
                     autocreate-cluster-domain-claims property is not true)"
                ));
            }
            registry.create(&domain, &ns);
        }
        Some(dc) if dc.namespace != ns => {
            dm.status.mark_domain_claim_not_owned();
            return Err(format!(
                "namespace {ns:?} does not own ClusterDomainClaim for {domain:?}"
            ));
        }
        Some(_) => {}
    }

    dm.status.mark_domain_claimed();
    Ok(())
}

/// A URI the cross-crate resolver produced for `spec.ref` (the Addressable
/// contract). cave-knative does not own the resolver — that walks the
/// Service/Route status — but it owns the *validation* of what comes back.
#[derive(Default, Debug, Clone)]
pub struct ResolvedUri {
    pub scheme: String,
    pub host: String,
    pub path: String,
}

impl ResolvedUri {
    /// Reconstruct the printable URL used in `MarkReferenceNotResolved`
    /// messages, matching the `%q` of upstream's `apis.URL.String()`.
    fn display(&self) -> String {
        let scheme = if self.scheme.is_empty() {
            "http"
        } else {
            &self.scheme
        };
        format!("{scheme}://{}{}", self.host, self.path)
    }
}

/// Port of `resolveRef`.
///
/// Validates the resolved Addressable URI and extracts the backend service
/// name for the KIngress. The resolved host must be the standard k8s service
/// DNS name `{name}.{namespace}.svc.{cluster-domain}`; DomainMapping does not
/// support path-based routing, non-service targets, or cross-namespace
/// references. On success sets `ReferenceResolved=True` and returns
/// `(host, backend_service)`; on failure sets `ReferenceResolved=False` with
/// the upstream message and returns `Err`.
pub fn resolve_ref(
    dm: &mut DomainMapping,
    resolved: &ResolvedUri,
    cluster_domain: &str,
) -> Result<(String, String), String> {
    // No path-based routing: a lone trailing slash is tolerated (TrimSuffix).
    if resolved.path.trim_end_matches('/') != "" {
        let msg = format!("resolved URI {:?} contains a path", resolved.display());
        dm.status.mark_reference_not_resolved(&msg);
        return Err(msg);
    }

    let required_suffix = format!(".svc.{cluster_domain}");
    let stripped = resolved.host.strip_suffix(&required_suffix);
    let parts: Vec<&str> = match stripped {
        Some(s) => s.split('.').collect(),
        None => Vec::new(),
    };
    if stripped.is_none() || parts.len() != 2 {
        let msg = format!(
            "resolved URI {:?} must be of the form {{name}}.{{namespace}}{required_suffix}",
            resolved.display()
        );
        dm.status.mark_reference_not_resolved(&msg);
        return Err(msg);
    }

    // Cross-namespace KIngress is unsupported.
    if parts[1] != dm.metadata.namespace {
        let msg = format!(
            "resolved URI {:?} must be in same namespace as DomainMapping",
            resolved.display()
        );
        dm.status.mark_reference_not_resolved(&msg);
        return Err(msg);
    }

    dm.status.mark_reference_resolved();
    Ok((resolved.host.clone(), parts[0].to_string()))
}

/// Port of `FinalizeKind` — clean up an autocreated ClusterDomainClaim when
/// the DomainMapping is deleted. No-op when autocreate is disabled (the
/// operator owns the claim lifecycle) or when the claim is owned by another
/// namespace (we must never delete someone else's claim).
pub fn finalize_kind(
    dm: &DomainMapping,
    registry: &mut DomainClaimRegistry,
    autocreate_cluster_domain_claims: bool,
) {
    if !autocreate_cluster_domain_claims {
        return;
    }
    let domain = &dm.metadata.name;
    match registry.get(domain) {
        Some(dc) if dc.namespace == dm.metadata.namespace => {
            registry.delete(domain);
        }
        _ => {}
    }
}
