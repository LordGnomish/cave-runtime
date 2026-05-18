// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
//! Service controller — provisions external load balancers for
//! `Service.spec.type == "LoadBalancer"` and finalises cleanup on delete.
//!
//! Upstream: [`pkg/controller/service`]. The full controller drives a
//! per-cloud `cloudprovider.LoadBalancer` interface; this scaffold isolates
//! the type-dispatch and finaliser logic.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub namespace: String,
    pub service_type: ServiceType,
    pub deletion_pending: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub lb_provisioned: bool,
    pub finalizer_present: bool,
}

/// Returns true iff the service type warrants this controller acting at all.
/// Mirrors `wantsLoadBalancer` in `pkg/controller/service/controller.go`.
pub fn wants_load_balancer(spec: &ServiceSpec) -> bool {
    spec.service_type == ServiceType::LoadBalancer
}

/// Mirrors `syncService` — the controller adds/removes the cloud finaliser
/// and provisions/destroys the LB.
pub fn reconcile(
    spec: &ServiceSpec,
    status: &ServiceStatus,
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    if !wants_load_balancer(spec) {
        return Ok(Reconcile::NoOp);
    }
    if spec.deletion_pending {
        // We must tear down the LB *before* removing the finaliser.
        if status.lb_provisioned {
            return Ok(Reconcile::Delete(1));
        }
        if status.finalizer_present {
            return Ok(Reconcile::Update(0));
        }
        return Ok(Reconcile::NoOp);
    }
    if !status.lb_provisioned {
        return Ok(Reconcile::Create(1));
    }
    Ok(Reconcile::NoOp)
}

// ── LoadBalancer attribute reconciliation ────────────────────────────────────
//
// Mirrors `cloudprovider.LoadBalancer.UpdateLoadBalancer` + the diff helpers
// in `pkg/controller/service/controller.go::needsUpdate`. The upstream
// controller compares the freshly-resolved Service spec against the cloud's
// current LB attributes and emits Update events for the cloudprovider to act
// on. We isolate the pure diff so callers can drive the cloud RPC.

/// `Service.spec.sessionAffinity` (Kubernetes core/v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionAffinity {
    None,
    ClientIp,
}

impl Default for SessionAffinity {
    fn default() -> Self {
        Self::None
    }
}

/// `Service.spec.externalTrafficPolicy` (Kubernetes core/v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExternalTrafficPolicy {
    Cluster,
    Local,
}

impl Default for ExternalTrafficPolicy {
    fn default() -> Self {
        Self::Cluster
    }
}

/// `Service.spec.ipFamilyPolicy` (Kubernetes core/v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IpFamilyPolicy {
    SingleStack,
    PreferDualStack,
    RequireDualStack,
}

impl Default for IpFamilyPolicy {
    fn default() -> Self {
        Self::SingleStack
    }
}

/// The attribute set that the cloud LB carries. Mirrors the per-platform
/// fields that `cloudprovider.LoadBalancer.UpdateLoadBalancer` re-applies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbAttributes {
    /// True when the LB is provisioned for internal (VPC-private) traffic
    /// rather than public access — backed by the
    /// `service.beta.kubernetes.io/aws-load-balancer-internal` annotation
    /// family.
    pub internal: bool,
    /// Pre-sorted CIDR allow-list — mirrors `Service.spec.loadBalancerSourceRanges`.
    pub source_ranges: Vec<String>,
    pub session_affinity: SessionAffinity,
    pub external_traffic_policy: ExternalTrafficPolicy,
    pub ip_family_policy: IpFamilyPolicy,
    /// `Service.spec.healthCheckNodePort` — only meaningful when
    /// `externalTrafficPolicy = Local`. Upstream guarantees the
    /// node-port is allocated by the apiserver, but the controller still
    /// needs to track changes (e.g., reset to zero on policy switch).
    pub health_check_node_port: Option<u16>,
    /// LB idle-timeout (seconds) — backed by the cloud-specific annotation
    /// (`service.beta.kubernetes.io/aws-load-balancer-connection-idle-timeout`,
    /// `cloud.google.com/load-balancer-timeout`, …). `None` means the cloud
    /// default applies and the controller must not emit an attribute change.
    pub idle_timeout_seconds: Option<u32>,
    /// Optional sticky logical pool the LB lives in — used by cloud
    /// implementations that pin an LB to a load-balancer pool / scheme /
    /// VPC. An empty string is treated as "unset" so callers don't need
    /// to fight `Option<String>` defaults.
    pub backend_pool: String,
}

/// Per-attribute diff between current cloud state and desired Service spec.
/// Useful for observability and for emitting structured PATCH bodies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbAttributeDiff {
    pub internal: bool,
    pub source_ranges: bool,
    pub session_affinity: bool,
    pub external_traffic_policy: bool,
    pub ip_family_policy: bool,
    pub health_check_node_port: bool,
    pub idle_timeout_seconds: bool,
    pub backend_pool: bool,
}

impl LbAttributeDiff {
    /// Total number of attribute fields that differ. Drives
    /// `Reconcile::Update(n)` (the count maps cleanly to "N PATCH ops").
    pub fn change_count(&self) -> u32 {
        [
            self.internal,
            self.source_ranges,
            self.session_affinity,
            self.external_traffic_policy,
            self.ip_family_policy,
            self.health_check_node_port,
            self.idle_timeout_seconds,
            self.backend_pool,
        ]
        .into_iter()
        .filter(|b| *b)
        .count() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.change_count() == 0
    }
}

/// Compute the per-field diff between `current` (cloud state) and `desired`
/// (Service spec). Source ranges are compared as *sets* — order does not
/// matter; the controller normalises before persisting.
pub fn diff_lb_attributes(current: &LbAttributes, desired: &LbAttributes) -> LbAttributeDiff {
    LbAttributeDiff {
        internal: current.internal != desired.internal,
        source_ranges: !source_ranges_equal(&current.source_ranges, &desired.source_ranges),
        session_affinity: current.session_affinity != desired.session_affinity,
        external_traffic_policy: current.external_traffic_policy != desired.external_traffic_policy,
        ip_family_policy: current.ip_family_policy != desired.ip_family_policy,
        health_check_node_port: current.health_check_node_port != desired.health_check_node_port,
        idle_timeout_seconds: current.idle_timeout_seconds != desired.idle_timeout_seconds,
        backend_pool: current.backend_pool != desired.backend_pool,
    }
}

fn source_ranges_equal(a: &[String], b: &[String]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut a_sorted: Vec<&String> = a.iter().collect();
    let mut b_sorted: Vec<&String> = b.iter().collect();
    a_sorted.sort();
    b_sorted.sort();
    a_sorted == b_sorted
}

/// Mirrors `cloudprovider.LoadBalancer.UpdateLoadBalancer`: given the cloud's
/// current attributes and the desired Service spec, decide what to do.
///
/// * `Reconcile::NoOp` — attributes match, no cloud RPC needed.
/// * `Reconcile::Update(n)` — emit a PATCH with n changed fields.
///
/// Validation:
/// * The Service must be `type=LoadBalancer` — anything else is a programmer
///   error (`InvalidSpec`) since the controller wouldn't dispatch otherwise.
/// * `healthCheckNodePort` may only be set when `externalTrafficPolicy=Local`
///   (upstream `validateHealthCheckNodePort` rejects this at admission, but
///   defence-in-depth is cheap).
pub fn sync_lb_attributes(
    spec: &ServiceSpec,
    current: &LbAttributes,
    desired: &LbAttributes,
) -> Result<Reconcile, ControllerError> {
    if spec.service_type != ServiceType::LoadBalancer {
        return Err(ControllerError::InvalidSpec {
            kind: "Service",
            reason: "sync_lb_attributes called on a non-LoadBalancer service".into(),
        });
    }
    if desired.health_check_node_port.is_some()
        && desired.external_traffic_policy != ExternalTrafficPolicy::Local
    {
        return Err(ControllerError::InvalidSpec {
            kind: "Service",
            reason: "healthCheckNodePort requires externalTrafficPolicy=Local".into(),
        });
    }
    let diff = diff_lb_attributes(current, desired);
    if diff.is_empty() {
        Ok(Reconcile::NoOp)
    } else {
        Ok(Reconcile::Update(diff.change_count()))
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/service/controller.go", "Controller");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn svc(t: ServiceType, deletion: bool) -> ServiceSpec {
        ServiceSpec {
            name: "frontend".into(),
            namespace: "default".into(),
            service_type: t,
            deletion_pending: deletion,
        }
    }

    #[test]
    fn cluster_ip_service_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "wantsLoadBalancer",
            "tenant-svc-clusterip"
        );
        let s = svc(ServiceType::ClusterIP, false);
        assert!(!wants_load_balancer(&s));
        assert_eq!(reconcile(&s, &ServiceStatus::default(), &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn missing_lb_triggers_create() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "ensureLoadBalancer",
            "tenant-svc-create-lb"
        );
        let s = svc(ServiceType::LoadBalancer, false);
        assert_eq!(reconcile(&s, &ServiceStatus::default(), &tenant).unwrap(), Reconcile::Create(1));
    }

    #[test]
    fn deletion_destroys_lb_before_finalizer_removal() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "tenant-svc-delete-lb"
        );
        let s = svc(ServiceType::LoadBalancer, true);
        let st = ServiceStatus { lb_provisioned: true, finalizer_present: true };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Delete(1));
    }

    #[test]
    fn finalizer_removal_emits_update_after_lb_gone() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "removeFinalizer",
            "tenant-svc-finalizer"
        );
        let s = svc(ServiceType::LoadBalancer, true);
        let st = ServiceStatus { lb_provisioned: false, finalizer_present: true };
        assert_eq!(reconcile(&s, &st, &tenant).unwrap(), Reconcile::Update(0));
    }

    // ── sync_lb_attributes ──────────────────────────────────────────────────

    fn lb_svc() -> ServiceSpec {
        svc(ServiceType::LoadBalancer, false)
    }

    #[test]
    fn lb_attributes_identical_are_a_noop() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-noop"
        );
        let attrs = LbAttributes::default();
        let res = sync_lb_attributes(&lb_svc(), &attrs, &attrs).unwrap();
        assert_eq!(res, Reconcile::NoOp);
    }

    #[test]
    fn lb_attributes_change_count_matches_field_diffs() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-diff"
        );
        let current = LbAttributes::default();
        let desired = LbAttributes {
            internal: true,
            source_ranges: vec!["10.0.0.0/8".into()],
            idle_timeout_seconds: Some(60),
            ..Default::default()
        };
        let res = sync_lb_attributes(&lb_svc(), &current, &desired).unwrap();
        // internal + source_ranges + idle_timeout = 3 field changes.
        assert_eq!(res, Reconcile::Update(3));
    }

    #[test]
    fn lb_attributes_source_ranges_order_independent() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-cidr-order"
        );
        let current = LbAttributes {
            source_ranges: vec!["10.0.0.0/8".into(), "192.168.0.0/16".into()],
            ..Default::default()
        };
        let desired = LbAttributes {
            source_ranges: vec!["192.168.0.0/16".into(), "10.0.0.0/8".into()],
            ..Default::default()
        };
        let res = sync_lb_attributes(&lb_svc(), &current, &desired).unwrap();
        assert_eq!(res, Reconcile::NoOp);
    }

    #[test]
    fn lb_attributes_session_affinity_diff_is_detected() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-sticky"
        );
        let current = LbAttributes::default();
        let desired = LbAttributes {
            session_affinity: SessionAffinity::ClientIp,
            ..Default::default()
        };
        assert_eq!(
            sync_lb_attributes(&lb_svc(), &current, &desired).unwrap(),
            Reconcile::Update(1)
        );
    }

    #[test]
    fn lb_attributes_health_check_requires_local_traffic_policy() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/api/v1/validation/validation.go",
            "validateHealthCheckNodePort",
            "tenant-lb-attr-hc-validate"
        );
        let current = LbAttributes::default();
        let desired = LbAttributes {
            health_check_node_port: Some(31_000),
            external_traffic_policy: ExternalTrafficPolicy::Cluster,
            ..Default::default()
        };
        let err = sync_lb_attributes(&lb_svc(), &current, &desired)
            .expect_err("healthCheckNodePort + Cluster policy must be rejected");
        match err {
            ControllerError::InvalidSpec { kind, .. } => assert_eq!(kind, "Service"),
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    #[test]
    fn lb_attributes_health_check_allowed_with_local_traffic_policy() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-hc-local"
        );
        let current = LbAttributes::default();
        let desired = LbAttributes {
            external_traffic_policy: ExternalTrafficPolicy::Local,
            health_check_node_port: Some(31_000),
            ..Default::default()
        };
        // externalTrafficPolicy + health_check_node_port = 2 changes.
        assert_eq!(
            sync_lb_attributes(&lb_svc(), &current, &desired).unwrap(),
            Reconcile::Update(2)
        );
    }

    #[test]
    fn lb_attributes_reject_non_lb_service_type() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-wrong-type"
        );
        let s = svc(ServiceType::ClusterIP, false);
        let attrs = LbAttributes::default();
        let err = sync_lb_attributes(&s, &attrs, &attrs)
            .expect_err("ClusterIP must not invoke attribute sync");
        match err {
            ControllerError::InvalidSpec { kind, .. } => assert_eq!(kind, "Service"),
            other => panic!("expected InvalidSpec, got {other:?}"),
        }
    }

    #[test]
    fn lb_attribute_diff_change_count_is_eight_when_all_fields_differ() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-all-diff"
        );
        let current = LbAttributes::default();
        let desired = LbAttributes {
            internal: true,
            source_ranges: vec!["10.0.0.0/8".into()],
            session_affinity: SessionAffinity::ClientIp,
            external_traffic_policy: ExternalTrafficPolicy::Local,
            ip_family_policy: IpFamilyPolicy::PreferDualStack,
            health_check_node_port: Some(31_000),
            idle_timeout_seconds: Some(120),
            backend_pool: "premium".into(),
        };
        let diff = diff_lb_attributes(&current, &desired);
        assert_eq!(diff.change_count(), 8);
        assert!(!diff.is_empty());
    }

    #[test]
    fn lb_attributes_serde_round_trip() {
        let (_cite, _tenant) = test_ctx!(
            "pkg/controller/service/controller.go",
            "needsUpdate",
            "tenant-lb-attr-serde"
        );
        let attrs = LbAttributes {
            internal: true,
            source_ranges: vec!["10.0.0.0/8".into()],
            session_affinity: SessionAffinity::ClientIp,
            external_traffic_policy: ExternalTrafficPolicy::Local,
            ip_family_policy: IpFamilyPolicy::RequireDualStack,
            health_check_node_port: Some(31_000),
            idle_timeout_seconds: Some(60),
            backend_pool: "standard".into(),
        };
        let s = serde_json::to_string(&attrs).unwrap();
        let back: LbAttributes = serde_json::from_str(&s).unwrap();
        assert_eq!(attrs, back);
    }
}
