// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Service registry — Cilium's view of K8s `Service` + `Endpoints`.
//!
//! Mirrors `pkg/service/service.go::svcInfo` and `pkg/loadbalancer/types.go`.
//! Each service has one or more `frontends` (ClusterIP + NodePort + ExternalIPs
//! + LoadBalancerIPs) all sharing the same backend pool. Backend health is
//! tracked separately and feeds [`super::lb::LoadBalancer`].
//!
//! Semantics (faithful to upstream):
//!
//! * `ClusterIP` is mandatory; everything else is optional.
//! * `NodePort` allocates a port from the host's NodePort range (default
//!   30000..=32767). Cilium also exposes the service on every node IP via
//!   `BPF_NF_CONNTRACK` rules.
//! * `ExternalIPs` are arbitrary user-provided VIPs that route to the
//!   service's backends.
//! * `LoadBalancerIPs` are typically assigned by an external LB (or the
//!   embedded `cilium-l2-announcer`).
//! * `dsr` (Direct Server Return) means the backend replies directly to
//!   the client; conntrack records the chosen backend so subsequent
//!   packets stick.
//! * `session_affinity = ClientIP` plus a `timeout_seconds` mirrors
//!   `Service.spec.sessionAffinityConfig`.

use crate::cilium::lb::{Algorithm, Backend as LbBackend, BackendState, LoadBalancer};
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

pub const NODEPORT_MIN: u16 = 30000;
pub const NODEPORT_MAX: u16 = 32767;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionAffinity {
    None,
    ClientIP,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: String,
    pub port: u16,
    pub target_port: u16,
    pub node_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub namespace: String,
    pub tenant: TenantId,
    pub service_type: ServiceType,
    pub cluster_ip: IpAddr,
    pub external_ips: Vec<IpAddr>,
    pub load_balancer_ips: Vec<IpAddr>,
    pub ports: Vec<ServicePort>,
    pub session_affinity: SessionAffinity,
    pub session_timeout: u64,
    pub dsr: bool,
    pub algorithm: Algorithm,
    pub backends: Vec<LbBackend>,
}

impl Service {
    pub fn cluster_ip(name: &str, ns: &str, tenant: TenantId, ip: IpAddr, port: u16) -> Self {
        Self {
            name: name.into(),
            namespace: ns.into(),
            tenant,
            service_type: ServiceType::ClusterIP,
            cluster_ip: ip,
            external_ips: Vec::new(),
            load_balancer_ips: Vec::new(),
            ports: vec![ServicePort {
                name: "default".into(),
                port,
                target_port: port,
                node_port: None,
            }],
            session_affinity: SessionAffinity::None,
            session_timeout: 0,
            dsr: false,
            algorithm: Algorithm::Random,
            backends: Vec::new(),
        }
    }
    pub fn key(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServiceError {
    #[error("nodeport {0} outside the {NODEPORT_MIN}..={NODEPORT_MAX} range")]
    NodePortOutOfRange(u16),
    #[error("nodeport {0} already in use")]
    NodePortInUse(u16),
    #[error("nodeport range exhausted")]
    NodePortExhausted,
    #[error("service `{0}` not found")]
    NotFound(String),
    #[error("tenant {tenant} cannot mutate service owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Default)]
pub struct ServiceRegistry {
    services: HashMap<String, Service>,
    /// Allocated NodePorts → owning service key.
    nodeports: HashMap<u16, String>,
    /// One LoadBalancer per (service, port-name).
    lbs: HashMap<(String, String), LoadBalancer>,
}

impl ServiceRegistry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.services.len()
    }
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    pub fn upsert(&mut self, svc: Service) -> Result<(), ServiceError> {
        // NodePort validation.
        for p in &svc.ports {
            if let Some(np) = p.node_port {
                if !(NODEPORT_MIN..=NODEPORT_MAX).contains(&np) {
                    return Err(ServiceError::NodePortOutOfRange(np));
                }
                if let Some(owner) = self.nodeports.get(&np) {
                    if owner != &svc.key() {
                        return Err(ServiceError::NodePortInUse(np));
                    }
                }
                self.nodeports.insert(np, svc.key());
            }
        }
        // Build/refresh per-port load balancers.
        for p in &svc.ports {
            let mut lb = LoadBalancer::new(
                svc.tenant.clone(),
                svc.algorithm,
                svc.backends
                    .iter()
                    .map(|b| LbBackend {
                        name: b.name.clone(),
                        ip: b.ip,
                        port: p.target_port,
                        state: b.state,
                        weight: b.weight,
                        open_connections: b.open_connections,
                    })
                    .collect(),
            );
            if matches!(svc.session_affinity, SessionAffinity::ClientIP) {
                lb.enable_client_ip_affinity(svc.session_timeout.max(1));
            }
            self.lbs.insert((svc.key(), p.name.clone()), lb);
        }
        self.services.insert(svc.key(), svc);
        Ok(())
    }

    pub fn lookup(&self, key: &str) -> Option<&Service> {
        self.services.get(key)
    }

    pub fn lookup_by_cluster_ip(&self, ip: IpAddr, port: u16) -> Option<&Service> {
        self.services
            .values()
            .find(|s| s.cluster_ip == ip && s.ports.iter().any(|p| p.port == port))
    }

    pub fn lookup_by_node_port(&self, np: u16) -> Option<&Service> {
        self.nodeports.get(&np).and_then(|k| self.services.get(k))
    }

    pub fn lookup_by_external_ip(&self, ip: IpAddr, port: u16) -> Option<&Service> {
        self.services.values().find(|s| {
            (s.external_ips.contains(&ip) || s.load_balancer_ips.contains(&ip))
                && s.ports.iter().any(|p| p.port == port)
        })
    }

    pub fn lb_for(&mut self, key: &str, port_name: &str) -> Option<&mut LoadBalancer> {
        self.lbs.get_mut(&(key.to_string(), port_name.to_string()))
    }

    pub fn remove(&mut self, key: &str) -> Result<(), ServiceError> {
        let svc = self
            .services
            .remove(key)
            .ok_or_else(|| ServiceError::NotFound(key.to_string()))?;
        for p in &svc.ports {
            if let Some(np) = p.node_port {
                self.nodeports.remove(&np);
            }
            self.lbs.remove(&(svc.key(), p.name.clone()));
        }
        Ok(())
    }

    /// Allocate a free NodePort. Mirrors the `NodePort` allocator in
    /// `pkg/loadbalancer/legacy/manager.go`.
    pub fn allocate_node_port(&self) -> Result<u16, ServiceError> {
        for p in NODEPORT_MIN..=NODEPORT_MAX {
            if !self.nodeports.contains_key(&p) {
                return Ok(p);
            }
        }
        Err(ServiceError::NodePortExhausted)
    }

    /// Update backend health for a service (e.g. EndpointSlice change).
    pub fn set_backend_state(
        &mut self,
        key: &str,
        backend_name: &str,
        state: BackendState,
    ) -> Result<(), ServiceError> {
        let svc = self
            .services
            .get_mut(key)
            .ok_or_else(|| ServiceError::NotFound(key.to_string()))?;
        for b in &mut svc.backends {
            if b.name == backend_name {
                b.state = state;
            }
        }
        // Replace LB backends so the change takes effect.
        let svc = self.services.get(key).unwrap().clone();
        for p in &svc.ports {
            if let Some(lb) = self.lbs.get_mut(&(svc.key(), p.name.clone())) {
                lb.replace_backends(
                    svc.backends
                        .iter()
                        .map(|b| LbBackend {
                            name: b.name.clone(),
                            ip: b.ip,
                            port: p.target_port,
                            state: b.state,
                            weight: b.weight,
                            open_connections: b.open_connections,
                        })
                        .collect(),
                );
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/service/service.go", "svcInfo");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::lb::FlowKey;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn fk(src: (u8, u8, u8, u8), sp: u16, dst: (u8, u8, u8, u8), dp: u16) -> FlowKey {
        FlowKey {
            src_ip: ip(src.0, src.1, src.2, src.3),
            src_port: sp,
            dst_ip: ip(dst.0, dst.1, dst.2, dst.3),
            dst_port: dp,
            proto: 6,
        }
    }

    fn basic_svc(tenant: TenantId) -> Service {
        let mut s = Service::cluster_ip("api", "default", tenant, ip(10, 96, 0, 1), 80);
        s.backends = vec![
            LbBackend::new("a", ip(10, 0, 1, 1), 8080),
            LbBackend::new("b", ip(10, 0, 1, 2), 8080),
        ];
        s
    }

    // ── ClusterIP ────────────────────────────────────────────────────────────

    #[test]
    fn svc_register_clusterip_service() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/service/service.go", "UpsertService", "tenant-svc-cip");
        let mut reg = ServiceRegistry::new();
        reg.upsert(basic_svc(tenant)).unwrap();
        assert_eq!(reg.len(), 1);
        let s = reg.lookup_by_cluster_ip(ip(10, 96, 0, 1), 80).unwrap();
        assert_eq!(s.service_type, ServiceType::ClusterIP);
    }

    #[test]
    fn svc_lookup_by_cluster_ip_with_wrong_port_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "LookupByFrontend",
            "tenant-svc-port"
        );
        let mut reg = ServiceRegistry::new();
        reg.upsert(basic_svc(tenant)).unwrap();
        assert!(reg.lookup_by_cluster_ip(ip(10, 96, 0, 1), 8080).is_none());
    }

    #[test]
    fn svc_remove_drops_service_and_its_lbs() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/service/service.go", "DeleteService", "tenant-svc-rm");
        let mut reg = ServiceRegistry::new();
        let svc = basic_svc(tenant);
        let key = svc.key();
        reg.upsert(svc).unwrap();
        reg.remove(&key).unwrap();
        assert!(reg.is_empty());
        assert!(reg.lb_for(&key, "default").is_none());
    }

    #[test]
    fn svc_remove_unknown_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/service/service.go",
            "DeleteService.NotFound",
            "tenant-svc-rmunk"
        );
        let mut reg = ServiceRegistry::new();
        let err = reg.remove("default/missing").unwrap_err();
        assert_eq!(err, ServiceError::NotFound("default/missing".into()));
    }

    // ── NodePort ─────────────────────────────────────────────────────────────

    #[test]
    fn svc_register_nodeport_in_range_succeeds() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "UpsertService.NodePort",
            "tenant-svc-np"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.service_type = ServiceType::NodePort;
        svc.ports[0].node_port = Some(30080);
        reg.upsert(svc).unwrap();
        let s = reg.lookup_by_node_port(30080).unwrap();
        assert_eq!(s.service_type, ServiceType::NodePort);
    }

    #[test]
    fn svc_register_nodeport_out_of_range_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "UpsertService.NodePort.Range",
            "tenant-svc-np-bad"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.service_type = ServiceType::NodePort;
        svc.ports[0].node_port = Some(80);
        let err = reg.upsert(svc).unwrap_err();
        assert_eq!(err, ServiceError::NodePortOutOfRange(80));
    }

    #[test]
    fn svc_register_nodeport_collision_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "UpsertService.NodePort.InUse",
            "tenant-svc-np-coll"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc1 = basic_svc(tenant.clone());
        svc1.service_type = ServiceType::NodePort;
        svc1.ports[0].node_port = Some(30080);
        reg.upsert(svc1).unwrap();
        let mut svc2 = Service::cluster_ip("other", "default", tenant, ip(10, 96, 0, 2), 80);
        svc2.service_type = ServiceType::NodePort;
        svc2.ports[0].node_port = Some(30080);
        let err = reg.upsert(svc2).unwrap_err();
        assert_eq!(err, ServiceError::NodePortInUse(30080));
    }

    #[test]
    fn svc_allocate_node_port_returns_first_free() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/legacy/manager.go",
            "allocateNodePort",
            "tenant-svc-np-alloc"
        );
        let reg = ServiceRegistry::new();
        let p = reg.allocate_node_port().unwrap();
        assert_eq!(p, NODEPORT_MIN);
    }

    #[test]
    fn svc_allocate_node_port_skips_in_use() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/loadbalancer/legacy/manager.go",
            "allocateNodePort.SkipInUse",
            "tenant-svc-np-skip"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.service_type = ServiceType::NodePort;
        svc.ports[0].node_port = Some(NODEPORT_MIN);
        reg.upsert(svc).unwrap();
        assert_eq!(reg.allocate_node_port().unwrap(), NODEPORT_MIN + 1);
    }

    // ── ExternalIPs / LoadBalancer ───────────────────────────────────────────

    #[test]
    fn svc_register_external_ip_lookup() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/service/service.go", "ExternalIP", "tenant-svc-ext");
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.external_ips = vec![ip(192, 0, 2, 100)];
        reg.upsert(svc).unwrap();
        let s = reg.lookup_by_external_ip(ip(192, 0, 2, 100), 80).unwrap();
        assert_eq!(s.name, "api");
    }

    #[test]
    fn svc_register_load_balancer_ip_lookup() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/service/service.go", "LoadBalancer", "tenant-svc-lb");
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.service_type = ServiceType::LoadBalancer;
        svc.load_balancer_ips = vec![ip(203, 0, 113, 50)];
        reg.upsert(svc).unwrap();
        let s = reg.lookup_by_external_ip(ip(203, 0, 113, 50), 80).unwrap();
        assert_eq!(s.service_type, ServiceType::LoadBalancer);
    }

    #[test]
    fn svc_external_ip_with_wrong_port_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "ExternalIP.Port",
            "tenant-svc-ext-port"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.external_ips = vec![ip(192, 0, 2, 100)];
        reg.upsert(svc).unwrap();
        assert!(reg
            .lookup_by_external_ip(ip(192, 0, 2, 100), 8080)
            .is_none());
    }

    // ── DSR / session affinity ───────────────────────────────────────────────

    #[test]
    fn svc_dsr_flag_persists_through_upsert() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/service/service.go", "ServiceFlagDSR", "tenant-svc-dsr");
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.dsr = true;
        reg.upsert(svc).unwrap();
        assert!(reg.lookup("default/api").unwrap().dsr);
    }

    #[test]
    fn svc_session_affinity_clientip_uses_lb_affinity_window() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "SessionAffinity.ClientIP",
            "tenant-svc-aff"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.session_affinity = SessionAffinity::ClientIP;
        svc.session_timeout = 60;
        svc.algorithm = Algorithm::RoundRobin;
        reg.upsert(svc).unwrap();
        let lb = reg.lb_for("default/api", "default").unwrap();
        let key = fk((10, 0, 0, 1), 1234, (10, 96, 0, 1), 80);
        let a = lb.select(key, 100).unwrap().name.clone();
        let b = lb
            .select(fk((10, 0, 0, 1), 5678, (10, 96, 0, 1), 80), 105)
            .unwrap()
            .name
            .clone();
        assert_eq!(a, b);
    }

    // ── Backend health ───────────────────────────────────────────────────────

    #[test]
    fn svc_set_backend_state_terminating_excludes_from_lb() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "UpdateBackendState",
            "tenant-svc-hlth"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.algorithm = Algorithm::RoundRobin;
        reg.upsert(svc).unwrap();
        reg.set_backend_state("default/api", "a", BackendState::Terminating)
            .unwrap();
        let lb = reg.lb_for("default/api", "default").unwrap();
        let mut hits: HashMap<String, u32> = HashMap::new();
        for sp in 1000..1010u16 {
            let n = lb
                .select(fk((10, 0, 0, 1), sp, (10, 96, 0, 1), 80), 100)
                .unwrap()
                .name
                .clone();
            *hits.entry(n).or_default() += 1;
        }
        assert_eq!(hits.get("a"), None);
        assert!(hits.contains_key("b"));
    }

    #[test]
    fn svc_set_backend_state_unknown_service_returns_not_found() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/service/service.go",
            "UpdateBackendState.NotFound",
            "tenant-svc-hlth-nf"
        );
        let mut reg = ServiceRegistry::new();
        let err = reg
            .set_backend_state("default/missing", "a", BackendState::Active)
            .unwrap_err();
        assert_eq!(err, ServiceError::NotFound("default/missing".into()));
    }

    // ── Lookup pathways ──────────────────────────────────────────────────────

    #[test]
    fn svc_lookup_by_node_port_returns_owning_service() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "LookupByNodePort",
            "tenant-svc-np-lk"
        );
        let mut reg = ServiceRegistry::new();
        let mut svc = basic_svc(tenant);
        svc.service_type = ServiceType::NodePort;
        svc.ports[0].node_port = Some(31000);
        reg.upsert(svc).unwrap();
        let s = reg.lookup_by_node_port(31000).unwrap();
        assert_eq!(s.cluster_ip, ip(10, 96, 0, 1));
    }

    #[test]
    fn svc_lookup_by_node_port_unknown_returns_none() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/service/service.go",
            "LookupByNodePort.None",
            "tenant-svc-np-none"
        );
        let reg = ServiceRegistry::new();
        assert!(reg.lookup_by_node_port(31999).is_none());
    }

    #[test]
    fn svc_upsert_idempotent_for_same_key() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/service/service.go",
            "UpsertService.Idempotent",
            "tenant-svc-idem"
        );
        let mut reg = ServiceRegistry::new();
        reg.upsert(basic_svc(tenant.clone())).unwrap();
        reg.upsert(basic_svc(tenant)).unwrap();
        assert_eq!(reg.len(), 1);
    }
}
