//! Cloud service controller — drives `Service.spec.type == LoadBalancer`
//! through a per-cloud `LoadBalancer` provider.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/controllers/service/controller.go`.
//! The in-tree `service` controller in `cave-controller-manager` decides what
//! to do; this controller is the per-cloud executor that asks the provider
//! to actually allocate or release the external IP.
//!
//! The deeper API in this module mirrors three upstream entry points:
//!
//! * `EnsureLoadBalancer` — create the LB if missing, returning the ingress
//!   address. Idempotent.
//! * `UpdateLoadBalancer` — re-write the LB's listeners / target set when
//!   the Service spec or NodePort assignment changes.
//! * `EnsureLoadBalancerDeleted` — drop the LB and release its address.
//!
//! Plus the upstream finalizer dance: the service controller adds
//! `service.kubernetes.io/load-balancer-cleanup` to the Service before
//! creating cloud resources, and removes it only after the cloud confirms
//! deletion.

use crate::provider::LoadBalancerIface;
use crate::types::{Cite, CloudError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// Finalizer key the upstream service controller writes to a `Service` to
/// prevent Kubernetes garbage-collection from removing the API object before
/// the cloud LB has been torn down.
pub const LB_CLEANUP_FINALIZER: &str = "service.kubernetes.io/load-balancer-cleanup";

/// Default load-balancer class. `None` on a Service means "the cluster
/// default" — this controller only touches Services whose
/// `spec.loadBalancerClass` is `None` or matches its own class.
pub const DEFAULT_LB_CLASS: &str = "kubernetes.io/default-class";

/// External-traffic policy enum — mirrors `core/v1.ServiceExternalTrafficPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExternalTrafficPolicy {
    /// Source IP preserved; only nodes hosting an endpoint receive traffic.
    Local,
    /// Cluster-wide load balancing; SNAT applied.
    Cluster,
}

impl ExternalTrafficPolicy {
    pub const fn key(self) -> &'static str {
        match self {
            ExternalTrafficPolicy::Local => "Local",
            ExternalTrafficPolicy::Cluster => "Cluster",
        }
    }
}

/// IP family policy. Mirrors `core/v1.IPFamilyPolicy`. `RequireDualStack`
/// fails fast if the cloud only supports one family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IpFamilyPolicy {
    SingleStack,
    PreferDualStack,
    RequireDualStack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IpFamily {
    V4,
    V6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    pub protocol: String,
    pub port: u16,
    pub target_port: u16,
    pub node_port: u16,
}

impl ServicePort {
    pub fn tcp(name: &str, port: u16, target_port: u16, node_port: u16) -> Self {
        Self {
            name: name.into(),
            protocol: "TCP".into(),
            port,
            target_port,
            node_port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSpec {
    pub name: String,
    pub namespace: String,
    pub ports: Vec<ServicePort>,
    pub external_traffic_policy: ExternalTrafficPolicy,
    pub health_check_node_port: Option<u16>,
    pub load_balancer_class: Option<String>,
    pub ip_family_policy: IpFamilyPolicy,
    pub ip_families: Vec<IpFamily>,
}

impl ServiceSpec {
    pub fn http(name: &str, namespace: &str) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            ports: vec![ServicePort::tcp("http", 80, 8080, 30080)],
            external_traffic_policy: ExternalTrafficPolicy::Cluster,
            health_check_node_port: None,
            load_balancer_class: None,
            ip_family_policy: IpFamilyPolicy::SingleStack,
            ip_families: vec![IpFamily::V4],
        }
    }

    /// Validate the Service against the cloud-provider rules upstream
    /// enforces in `validateServiceLBStatus` + `apiserver` admission.
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.ports.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: crate::types::ProviderName::Hetzner,
                reason: format!("service {} has no ports", self.name),
            });
        }
        if self.external_traffic_policy == ExternalTrafficPolicy::Local
            && self.health_check_node_port.is_none()
        {
            return Err(CloudError::InvalidConfig {
                provider: crate::types::ProviderName::Hetzner,
                reason: format!(
                    "service {}: ExternalTrafficPolicy=Local requires a healthCheckNodePort",
                    self.name
                ),
            });
        }
        if self.ip_family_policy == IpFamilyPolicy::RequireDualStack
            && self.ip_families.len() < 2
        {
            return Err(CloudError::InvalidConfig {
                provider: crate::types::ProviderName::Hetzner,
                reason: format!(
                    "service {}: ipFamilyPolicy=RequireDualStack but only {} families specified",
                    self.name,
                    self.ip_families.len()
                ),
            });
        }
        Ok(())
    }
}

/// LoadBalancer status — the `Service.status.loadBalancer.ingress[*]` shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadBalancerStatus {
    pub ingress_ip: Option<String>,
    pub ingress_hostname: Option<String>,
}

impl LoadBalancerStatus {
    pub fn empty() -> Self {
        Self { ingress_ip: None, ingress_hostname: None }
    }

    pub fn ip(addr: impl Into<String>) -> Self {
        Self { ingress_ip: Some(addr.into()), ingress_hostname: None }
    }

    pub fn hostname(host: impl Into<String>) -> Self {
        Self { ingress_ip: None, ingress_hostname: Some(host.into()) }
    }

    pub fn is_published(&self) -> bool {
        self.ingress_ip.is_some() || self.ingress_hostname.is_some()
    }
}

/// Snapshot of what the controller observes for a single Service. Mirrors
/// the fields read by `syncLoadBalancerIfNeeded` upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceObservation {
    pub spec: ServiceSpec,
    pub status: LoadBalancerStatus,
    /// True when the Service has a non-zero `deletionTimestamp` — the cloud
    /// resources must be torn down before the finalizer is dropped.
    pub deletion_pending: bool,
    /// Last spec we successfully programmed onto the cloud. `None` until the
    /// first `EnsureLoadBalancer` succeeds.
    pub last_applied_spec: Option<ServiceSpec>,
    /// Whether the LB-cleanup finalizer is present on the Service.
    pub finalizer_present: bool,

    // Backwards-compatible projection of the v1 API used by the original
    // tests in this crate. Kept so `ServiceObservation::from_v1` still works.
    #[serde(default)]
    pub legacy_external_ip: Option<String>,
}

impl ServiceObservation {
    /// Backwards-compatibility constructor. Mirrors the original v1 shape so
    /// the v1 reconcile path keeps working without touching call sites.
    pub fn from_v1(name: &str, namespace: &str, ip: Option<&str>, deletion: bool) -> Self {
        let spec = ServiceSpec {
            name: name.into(),
            namespace: namespace.into(),
            ports: vec![ServicePort::tcp("http", 80, 8080, 30080)],
            external_traffic_policy: ExternalTrafficPolicy::Cluster,
            health_check_node_port: None,
            load_balancer_class: None,
            ip_family_policy: IpFamilyPolicy::SingleStack,
            ip_families: vec![IpFamily::V4],
        };
        let status = match ip {
            Some(addr) => LoadBalancerStatus::ip(addr),
            None => LoadBalancerStatus::empty(),
        };
        Self {
            spec,
            status,
            deletion_pending: deletion,
            last_applied_spec: None,
            finalizer_present: !deletion && ip.is_some(),
            legacy_external_ip: ip.map(|s| s.to_string()),
        }
    }

    /// Convenience for the legacy field used by the original v1 reconcile.
    pub fn external_ip(&self) -> Option<&str> {
        self.status.ingress_ip.as_deref().or(self.legacy_external_ip.as_deref())
    }

    /// Convenience for the legacy field used by the original v1 reconcile.
    pub fn service_name(&self) -> &str {
        &self.spec.name
    }

    /// Convenience for the legacy field used by the original v1 reconcile.
    pub fn namespace(&self) -> &str {
        &self.spec.namespace
    }
}

// ─── Spec drift ──────────────────────────────────────────────────────────────

/// True iff the LB-relevant subset of `spec` differs from `last_applied`.
/// Mirrors the comparison in `Controller.processServiceUpdate`.
pub fn lb_spec_drifted(current: &ServiceSpec, last_applied: &ServiceSpec) -> bool {
    current.ports != last_applied.ports
        || current.external_traffic_policy != last_applied.external_traffic_policy
        || current.health_check_node_port != last_applied.health_check_node_port
        || current.ip_family_policy != last_applied.ip_family_policy
        || current.ip_families != last_applied.ip_families
}

/// True iff this controller should manage the Service's LB. Mirrors the
/// `loadBalancerClass` short-circuit in `syncLoadBalancerIfNeeded`.
pub fn should_manage(spec: &ServiceSpec, our_class: &str) -> bool {
    match &spec.load_balancer_class {
        None => true,
        Some(cls) => cls == our_class,
    }
}

// ─── Lifecycle phases ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbPhase {
    /// Nothing to do.
    NoOp,
    /// Add the cleanup finalizer before reaching out to the cloud.
    AddFinalizer,
    /// Call `EnsureLoadBalancer` — the LB doesn't exist yet.
    Ensure,
    /// Call `UpdateLoadBalancer` — the LB exists but its program drifted.
    Update,
    /// Call `EnsureLoadBalancerDeleted` — the Service is being deleted.
    Delete,
    /// Cloud delete succeeded; remove the cleanup finalizer.
    RemoveFinalizer,
    /// Service is unmanaged (mismatched loadBalancerClass).
    Skip,
}

/// Decide which lifecycle phase the controller should run for `obs`. Pure
/// function — does not touch the provider.
pub fn next_phase(obs: &ServiceObservation, our_class: &str) -> LbPhase {
    if !should_manage(&obs.spec, our_class) {
        return LbPhase::Skip;
    }
    if obs.deletion_pending {
        if obs.status.is_published() {
            return LbPhase::Delete;
        }
        if obs.finalizer_present {
            return LbPhase::RemoveFinalizer;
        }
        return LbPhase::NoOp;
    }
    if !obs.finalizer_present {
        return LbPhase::AddFinalizer;
    }
    if !obs.status.is_published() {
        return LbPhase::Ensure;
    }
    match &obs.last_applied_spec {
        Some(prev) if lb_spec_drifted(&obs.spec, prev) => LbPhase::Update,
        _ => LbPhase::NoOp,
    }
}

// ─── Reconcile (v1 + v2) ─────────────────────────────────────────────────────

/// V1 reconcile — kept for backwards compatibility with the original
/// scaffold. Mirrors `syncLoadBalancerIfNeeded` upstream.
pub fn reconcile<P: LoadBalancerIface>(
    provider: &P,
    obs: &ServiceObservation,
    tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    provider.authorise(tenant, "Service", obs.service_name())?;
    if obs.deletion_pending {
        if obs.external_ip().is_some() {
            provider.delete_lb(tenant, obs.service_name())?;
            return Ok(Reconcile::Delete(1));
        }
        return Ok(Reconcile::NoOp);
    }
    if obs.external_ip().is_none() {
        let _ip = provider.ensure_lb(tenant, obs.service_name())?;
        return Ok(Reconcile::AllocateIp(1));
    }
    Ok(Reconcile::NoOp)
}

/// V2 reconcile — picks a phase, runs the matching provider call. The
/// phase-only return value lets tests reason about lifecycle without poking
/// at counters.
pub fn reconcile_phase<P: LoadBalancerIface>(
    provider: &P,
    obs: &ServiceObservation,
    our_class: &str,
    tenant: &TenantId,
) -> Result<LbPhase, CloudError> {
    let phase = next_phase(obs, our_class);
    match phase {
        LbPhase::Ensure => {
            provider.authorise(tenant, "Service", &obs.spec.name)?;
            obs.spec.validate()?;
            let _ip = provider.ensure_lb(tenant, &obs.spec.name)?;
        }
        LbPhase::Update => {
            provider.authorise(tenant, "Service", &obs.spec.name)?;
            obs.spec.validate()?;
            // Idempotent — provider stub maps Update to ensure_lb.
            let _ip = provider.ensure_lb(tenant, &obs.spec.name)?;
        }
        LbPhase::Delete => {
            provider.authorise(tenant, "Service", &obs.spec.name)?;
            provider.delete_lb(tenant, &obs.spec.name)?;
        }
        LbPhase::AddFinalizer
        | LbPhase::RemoveFinalizer
        | LbPhase::NoOp
        | LbPhase::Skip => {}
    }
    Ok(phase)
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s(
    "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
    "Controller",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CloudConfig, CloudProvider};
    use crate::test_ctx;
    use crate::types::{ProviderName, TenantId};
    use std::cell::Cell;

    /// Minimal in-memory LB provider for unit tests.
    struct StubLb {
        cfg: CloudConfig,
        ensured: Cell<u32>,
        deleted: Cell<u32>,
    }

    impl StubLb {
        fn new(tenant: &str) -> Self {
            Self {
                cfg: CloudConfig {
                    tenant: TenantId::new(tenant).expect("test fixture"),
                    provider: ProviderName::Hetzner,
                    region: "fsn1".into(),
                    credential_ref: "vault://kv/hcloud".into(),
                },
                ensured: Cell::new(0),
                deleted: Cell::new(0),
            }
        }
    }
    impl CloudProvider for StubLb {
        fn name(&self) -> ProviderName {
            self.cfg.provider
        }
        fn config(&self) -> &CloudConfig {
            &self.cfg
        }
    }
    impl LoadBalancerIface for StubLb {
        fn ensure_lb(&self, _t: &TenantId, _svc: &str) -> Result<String, CloudError> {
            self.ensured.set(self.ensured.get() + 1);
            Ok("203.0.113.7".into())
        }
        fn delete_lb(&self, _t: &TenantId, _svc: &str) -> Result<(), CloudError> {
            self.deleted.set(self.deleted.get() + 1);
            Ok(())
        }
    }

    fn obs(name: &str, ip: Option<&str>, deletion: bool) -> ServiceObservation {
        ServiceObservation::from_v1(name, "default", ip, deletion)
    }

    fn full_obs(spec: ServiceSpec, ip: Option<&str>, deletion: bool, finalizer: bool) -> ServiceObservation {
        ServiceObservation {
            spec,
            status: ip.map(LoadBalancerStatus::ip).unwrap_or_else(LoadBalancerStatus::empty),
            deletion_pending: deletion,
            last_applied_spec: None,
            finalizer_present: finalizer,
            legacy_external_ip: ip.map(|s| s.to_string()),
        }
    }

    // ─── V1 reconcile (existing) ─────────────────────────────────────────────

    #[test]
    fn missing_external_ip_calls_ensure_lb() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", None, false), &tenant).unwrap();
        assert_eq!(r, Reconcile::AllocateIp(1));
        assert_eq!(p.ensured.get(), 1);
    }

    #[test]
    fn pre_allocated_ip_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", Some("203.0.113.1"), false), &tenant).unwrap();
        assert_eq!(r, Reconcile::NoOp);
        assert_eq!(p.ensured.get(), 0);
    }

    #[test]
    fn deletion_with_ip_calls_delete_lb() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancerDeleted",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", Some("203.0.113.1"), true), &tenant).unwrap();
        assert_eq!(r, Reconcile::Delete(1));
        assert_eq!(p.deleted.get(), 1);
    }

    #[test]
    fn deletion_without_ip_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancerDeleted",
            "acme"
        );
        let p = StubLb::new("acme");
        let r = reconcile(&p, &obs("web", None, true), &tenant).unwrap();
        assert_eq!(r, Reconcile::NoOp);
        assert_eq!(p.deleted.get(), 0);
    }

    #[test]
    fn cross_tenant_call_is_refused_before_provider_is_invoked() {
        let (_cite, attacker) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "Controller",
            "tenant-attacker"
        );
        let p = StubLb::new("acme");
        let err = reconcile(&p, &obs("web", None, false), &attacker).unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
        assert_eq!(p.ensured.get(), 0);
    }

    // ─── ServiceSpec validation ──────────────────────────────────────────────

    #[test]
    fn service_spec_http_constructor_is_valid() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "validateService",
            "tenant-svc-http"
        );
        assert!(ServiceSpec::http("web", "default").validate().is_ok());
    }

    #[test]
    fn service_spec_with_zero_ports_is_invalid() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "validateService",
            "tenant-svc-noport"
        );
        let mut s = ServiceSpec::http("web", "default");
        s.ports.clear();
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn etp_local_requires_health_check_node_port() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "validateService",
            "tenant-svc-local"
        );
        let mut s = ServiceSpec::http("web", "default");
        s.external_traffic_policy = ExternalTrafficPolicy::Local;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        s.health_check_node_port = Some(31000);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn require_dual_stack_with_one_family_is_invalid() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "validateIPFamilyPolicy",
            "tenant-svc-dual"
        );
        let mut s = ServiceSpec::http("web", "default");
        s.ip_family_policy = IpFamilyPolicy::RequireDualStack;
        assert!(matches!(s.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
        s.ip_families = vec![IpFamily::V4, IpFamily::V6];
        assert!(s.validate().is_ok());
    }

    #[test]
    fn external_traffic_policy_keys_match_upstream() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "ServiceExternalTrafficPolicy",
            "tenant-svc-etp-keys"
        );
        assert_eq!(ExternalTrafficPolicy::Local.key(), "Local");
        assert_eq!(ExternalTrafficPolicy::Cluster.key(), "Cluster");
    }

    // ─── LB status ───────────────────────────────────────────────────────────

    #[test]
    fn lb_status_empty_constructor_is_unpublished() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "LoadBalancerStatus",
            "tenant-lb-empty"
        );
        let s = LoadBalancerStatus::empty();
        assert!(!s.is_published());
    }

    #[test]
    fn lb_status_with_ip_is_published() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "LoadBalancerIngress",
            "tenant-lb-ip"
        );
        let s = LoadBalancerStatus::ip("203.0.113.5");
        assert!(s.is_published());
        assert_eq!(s.ingress_ip.as_deref(), Some("203.0.113.5"));
        assert!(s.ingress_hostname.is_none());
    }

    #[test]
    fn lb_status_with_hostname_is_published() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "LoadBalancerIngress",
            "tenant-lb-host"
        );
        let s = LoadBalancerStatus::hostname("lb.example.com");
        assert!(s.is_published());
        assert!(s.ingress_ip.is_none());
        assert_eq!(s.ingress_hostname.as_deref(), Some("lb.example.com"));
    }

    // ─── Spec drift ──────────────────────────────────────────────────────────

    #[test]
    fn lb_spec_drifted_returns_false_for_identical_specs() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "needsUpdate",
            "tenant-drift-same"
        );
        let s = ServiceSpec::http("web", "default");
        assert!(!lb_spec_drifted(&s, &s));
    }

    #[test]
    fn lb_spec_drifted_detects_port_change() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "needsUpdate",
            "tenant-drift-port"
        );
        let prev = ServiceSpec::http("web", "default");
        let mut now = prev.clone();
        now.ports[0].port = 443;
        assert!(lb_spec_drifted(&now, &prev));
    }

    #[test]
    fn lb_spec_drifted_detects_etp_change() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "needsUpdate",
            "tenant-drift-etp"
        );
        let prev = ServiceSpec::http("web", "default");
        let mut now = prev.clone();
        now.external_traffic_policy = ExternalTrafficPolicy::Local;
        now.health_check_node_port = Some(31000);
        assert!(lb_spec_drifted(&now, &prev));
    }

    #[test]
    fn lb_spec_drifted_detects_health_check_node_port_change() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "needsUpdate",
            "tenant-drift-hc"
        );
        let mut prev = ServiceSpec::http("web", "default");
        prev.external_traffic_policy = ExternalTrafficPolicy::Local;
        prev.health_check_node_port = Some(31000);
        let mut now = prev.clone();
        now.health_check_node_port = Some(31002);
        assert!(lb_spec_drifted(&now, &prev));
    }

    #[test]
    fn lb_spec_drifted_detects_ip_family_change() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "needsUpdate",
            "tenant-drift-ipfam"
        );
        let prev = ServiceSpec::http("web", "default");
        let mut now = prev.clone();
        now.ip_families = vec![IpFamily::V6];
        assert!(lb_spec_drifted(&now, &prev));
    }

    #[test]
    fn lb_spec_drifted_ignores_name_change() {
        // Renames are handled at a higher level; the LB itself is keyed off
        // (namespace, name), so the controller does not re-program for them.
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "needsUpdate",
            "tenant-drift-name"
        );
        let prev = ServiceSpec::http("web", "default");
        let mut now = prev.clone();
        now.name = "web2".into();
        assert!(!lb_spec_drifted(&now, &prev));
    }

    // ─── should_manage ───────────────────────────────────────────────────────

    #[test]
    fn should_manage_when_load_balancer_class_is_unset() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "shouldSyncLoadBalancer",
            "tenant-mgr-default"
        );
        let s = ServiceSpec::http("web", "default");
        assert!(should_manage(&s, DEFAULT_LB_CLASS));
    }

    #[test]
    fn should_manage_when_load_balancer_class_matches() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "shouldSyncLoadBalancer",
            "tenant-mgr-match"
        );
        let mut s = ServiceSpec::http("web", "default");
        s.load_balancer_class = Some(DEFAULT_LB_CLASS.into());
        assert!(should_manage(&s, DEFAULT_LB_CLASS));
    }

    #[test]
    fn should_not_manage_when_load_balancer_class_differs() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "shouldSyncLoadBalancer",
            "tenant-mgr-skip"
        );
        let mut s = ServiceSpec::http("web", "default");
        s.load_balancer_class = Some("other.io/private".into());
        assert!(!should_manage(&s, DEFAULT_LB_CLASS));
    }

    // ─── Phase decisions ─────────────────────────────────────────────────────

    #[test]
    fn next_phase_is_skip_for_unmanaged_class() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "tenant-phase-skip"
        );
        let mut s = ServiceSpec::http("web", "default");
        s.load_balancer_class = Some("other.io/x".into());
        let o = full_obs(s, None, false, false);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::Skip);
    }

    #[test]
    fn next_phase_is_add_finalizer_when_missing() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "addFinalizer",
            "tenant-phase-final"
        );
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, false, false);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::AddFinalizer);
    }

    #[test]
    fn next_phase_is_ensure_when_finalizer_set_but_status_empty() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancer",
            "tenant-phase-ensure"
        );
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, false, true);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::Ensure);
    }

    #[test]
    fn next_phase_is_noop_when_status_published_and_spec_matches() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "tenant-phase-noop"
        );
        let spec = ServiceSpec::http("web", "default");
        let mut o = full_obs(spec.clone(), Some("203.0.113.1"), false, true);
        o.last_applied_spec = Some(spec);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::NoOp);
    }

    #[test]
    fn next_phase_is_update_when_spec_drifted() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "updateLoadBalancer",
            "tenant-phase-update"
        );
        let prev = ServiceSpec::http("web", "default");
        let mut now = prev.clone();
        now.ports[0].port = 443;
        let mut o = full_obs(now, Some("203.0.113.1"), false, true);
        o.last_applied_spec = Some(prev);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::Update);
    }

    #[test]
    fn next_phase_is_delete_when_deletion_pending_and_published() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancerDeleted",
            "tenant-phase-del"
        );
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, Some("203.0.113.1"), true, true);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::Delete);
    }

    #[test]
    fn next_phase_is_remove_finalizer_after_cloud_delete() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "removeFinalizer",
            "tenant-phase-rmfin"
        );
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, true, true);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::RemoveFinalizer);
    }

    #[test]
    fn next_phase_is_noop_when_deletion_pending_with_no_finalizer_and_no_status() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "tenant-phase-del-noop"
        );
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, true, false);
        assert_eq!(next_phase(&o, DEFAULT_LB_CLASS), LbPhase::NoOp);
    }

    // ─── reconcile_phase wiring ──────────────────────────────────────────────

    #[test]
    fn reconcile_phase_ensure_invokes_provider_ensure() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancer",
            "acme"
        );
        let p = StubLb::new("acme");
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, false, true);
        let phase = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap();
        assert_eq!(phase, LbPhase::Ensure);
        assert_eq!(p.ensured.get(), 1);
        assert_eq!(p.deleted.get(), 0);
    }

    #[test]
    fn reconcile_phase_update_invokes_provider_ensure() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "updateLoadBalancer",
            "acme"
        );
        let p = StubLb::new("acme");
        let prev = ServiceSpec::http("web", "default");
        let mut now = prev.clone();
        now.ports[0].port = 443;
        let mut o = full_obs(now, Some("203.0.113.1"), false, true);
        o.last_applied_spec = Some(prev);
        let phase = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap();
        assert_eq!(phase, LbPhase::Update);
        assert_eq!(p.ensured.get(), 1);
    }

    #[test]
    fn reconcile_phase_delete_invokes_provider_delete() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancerDeleted",
            "acme"
        );
        let p = StubLb::new("acme");
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, Some("203.0.113.1"), true, true);
        let phase = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap();
        assert_eq!(phase, LbPhase::Delete);
        assert_eq!(p.deleted.get(), 1);
    }

    #[test]
    fn reconcile_phase_skip_does_not_invoke_provider() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let p = StubLb::new("acme");
        let mut s = ServiceSpec::http("web", "default");
        s.load_balancer_class = Some("other.io/x".into());
        let o = full_obs(s, None, false, false);
        let phase = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap();
        assert_eq!(phase, LbPhase::Skip);
        assert_eq!(p.ensured.get(), 0);
        assert_eq!(p.deleted.get(), 0);
    }

    #[test]
    fn reconcile_phase_finalizer_phase_does_not_invoke_provider() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "addFinalizer",
            "acme"
        );
        let p = StubLb::new("acme");
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, false, false);
        let phase = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap();
        assert_eq!(phase, LbPhase::AddFinalizer);
        assert_eq!(p.ensured.get(), 0);
    }

    #[test]
    fn reconcile_phase_noop_does_not_invoke_provider() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "syncLoadBalancerIfNeeded",
            "acme"
        );
        let p = StubLb::new("acme");
        let spec = ServiceSpec::http("web", "default");
        let mut o = full_obs(spec.clone(), Some("203.0.113.1"), false, true);
        o.last_applied_spec = Some(spec);
        let phase = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap();
        assert_eq!(phase, LbPhase::NoOp);
        assert_eq!(p.ensured.get(), 0);
        assert_eq!(p.deleted.get(), 0);
    }

    #[test]
    fn reconcile_phase_ensure_validates_spec_first() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ensureLoadBalancer",
            "acme"
        );
        let p = StubLb::new("acme");
        let mut s = ServiceSpec::http("web", "default");
        s.external_traffic_policy = ExternalTrafficPolicy::Local; // missing HC port
        let o = full_obs(s, None, false, true);
        let err = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &tenant).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        assert_eq!(p.ensured.get(), 0);
    }

    #[test]
    fn reconcile_phase_refuses_cross_tenant_calls() {
        let (_cite, attacker) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "Controller",
            "tenant-attacker"
        );
        let p = StubLb::new("acme");
        let s = ServiceSpec::http("web", "default");
        let o = full_obs(s, None, false, true);
        let err = reconcile_phase(&p, &o, DEFAULT_LB_CLASS, &attacker).unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
        assert_eq!(p.ensured.get(), 0);
    }

    // ─── Finalizer constant ──────────────────────────────────────────────────

    #[test]
    fn finalizer_constant_matches_upstream() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/service/controller.go",
            "ServiceLoadBalancerFinalizer",
            "tenant-finalizer-key"
        );
        assert_eq!(LB_CLEANUP_FINALIZER, "service.kubernetes.io/load-balancer-cleanup");
    }

    // ─── ServicePort helpers ─────────────────────────────────────────────────

    #[test]
    fn service_port_tcp_helper_uses_capital_tcp() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "ServicePort",
            "tenant-svc-port"
        );
        let p = ServicePort::tcp("http", 80, 8080, 30080);
        assert_eq!(p.protocol, "TCP");
        assert_eq!(p.port, 80);
        assert_eq!(p.target_port, 8080);
        assert_eq!(p.node_port, 30080);
    }

    // ─── ServiceObservation helpers ──────────────────────────────────────────

    #[test]
    fn observation_external_ip_prefers_status_over_legacy() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "ServiceStatus",
            "tenant-svc-obs-ip"
        );
        let s = ServiceSpec::http("web", "default");
        let mut o = full_obs(s, Some("203.0.113.1"), false, true);
        o.status = LoadBalancerStatus::ip("198.51.100.5");
        assert_eq!(o.external_ip(), Some("198.51.100.5"));
    }

    #[test]
    fn observation_external_ip_falls_back_to_legacy_when_status_empty() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "ServiceStatus",
            "tenant-svc-obs-legacy"
        );
        let mut o = ServiceObservation::from_v1("web", "default", Some("203.0.113.1"), false);
        o.status = LoadBalancerStatus::empty();
        assert_eq!(o.external_ip(), Some("203.0.113.1"));
    }

    #[test]
    fn observation_namespace_and_service_name_round_trip() {
        let (_cite, _t) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/types.go",
            "ObjectMeta",
            "tenant-svc-obs-meta"
        );
        let o = ServiceObservation::from_v1("api", "platform", None, false);
        assert_eq!(o.service_name(), "api");
        assert_eq!(o.namespace(), "platform");
    }
}
