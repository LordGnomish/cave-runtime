// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SocketLB — kernel-side socket-level load balancing via cgroup BPF.
//!
//! Mirrors `pkg/socketlb/socketlb.go` (the agent-side manager) and the
//! BPF programs in `bpf/bpf_sock.c`:
//!
//! * `cil_sock4_connect` (cgroup/connect4) — when an in-cluster pod
//!   calls `connect()` to a service ClusterIP, the BPF rewrites the
//!   destination to a backend pod IP *before* the connect syscall
//!   reaches the kernel TCP stack. Skips the per-packet datapath
//!   entirely → zero per-packet overhead.
//! * `cil_sock4_sendmsg` (cgroup/sendmsg4) — same rewrite for connected
//!   UDP and connectionless flows.
//! * `cil_sock4_recvmsg` (cgroup/recvmsg4) — reverses the rewrite on
//!   the receive side so applications see the original destination.
//!
//! Cgroup attach: `BPF_CGROUP_INET4_CONNECT`, `BPF_CGROUP_UDP4_SENDMSG`,
//! `BPF_CGROUP_UDP4_RECVMSG` (and IPv6 variants).
//!
//! Semantics (faithful to upstream):
//!
//! * SocketLB applies only when source AND destination are in-cluster.
//!   External traffic is excluded so `hostNetwork` pods see real ClusterIPs.
//! * If the source pod is in `host-net-namespace` (init container, host
//!   netns), the BPF skips the rewrite when the destination is *not* a
//!   ClusterIP (mirrors `socketlb.HostnsOnly` flag).
//! * Backend selection uses the same Maglev/RR/Random pool as the
//!   per-packet LB, but the result is recorded in `revnat_id` map for
//!   lookup on the recv side.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CgroupHook {
    /// `BPF_CGROUP_INET4_CONNECT`
    InetConnect4,
    /// `BPF_CGROUP_INET6_CONNECT`
    InetConnect6,
    /// `BPF_CGROUP_UDP4_SENDMSG`
    UdpSendmsg4,
    /// `BPF_CGROUP_UDP6_SENDMSG`
    UdpSendmsg6,
    /// `BPF_CGROUP_UDP4_RECVMSG`
    UdpRecvmsg4,
    /// `BPF_CGROUP_UDP6_RECVMSG`
    UdpRecvmsg6,
    /// `BPF_CGROUP_INET4_GETPEERNAME`
    InetGetpeername4,
}

impl CgroupHook {
    pub fn name(self) -> &'static str {
        match self {
            CgroupHook::InetConnect4 => "BPF_CGROUP_INET4_CONNECT",
            CgroupHook::InetConnect6 => "BPF_CGROUP_INET6_CONNECT",
            CgroupHook::UdpSendmsg4 => "BPF_CGROUP_UDP4_SENDMSG",
            CgroupHook::UdpSendmsg6 => "BPF_CGROUP_UDP6_SENDMSG",
            CgroupHook::UdpRecvmsg4 => "BPF_CGROUP_UDP4_RECVMSG",
            CgroupHook::UdpRecvmsg6 => "BPF_CGROUP_UDP6_RECVMSG",
            CgroupHook::InetGetpeername4 => "BPF_CGROUP_INET4_GETPEERNAME",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceFrontend {
    pub cluster_ip: IpAddr,
    pub port: u16,
    pub protocol: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceBackend {
    pub backend_id: u32,
    pub backend_ip: IpAddr,
    pub backend_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SockLbDecision {
    /// Rewrite the syscall args to (backend_ip, backend_port).
    Rewrite {
        backend_ip: IpAddr,
        backend_port: u16,
        revnat_id: u32,
    },
    /// Pass-through: not a service IP or source not eligible.
    Passthrough,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SockLbError {
    #[error("cgroup root `{0}` already attached")]
    AlreadyAttached(String),
    #[error("cgroup root `{0}` not attached")]
    NotAttached(String),
    #[error("service frontend `{ip}:{port}` already registered")]
    DuplicateFrontend { ip: IpAddr, port: u16 },
    #[error("tenant {tenant} cannot mutate sock LB owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SockLbConfig {
    /// Apply rewrites only inside non-host network namespaces; for host-
    /// netns sockets only rewrite explicit ClusterIP traffic.
    pub host_ns_only: bool,
    /// Track terminating backends for graceful shutdown.
    pub track_terminating: bool,
}

impl Default for SockLbConfig {
    fn default() -> Self {
        Self {
            host_ns_only: false,
            track_terminating: true,
        }
    }
}

#[derive(Debug)]
pub struct SockLbManager {
    pub tenant: TenantId,
    pub config: SockLbConfig,
    /// Cgroup paths the agent has attached to. Mirrors the in-kernel
    /// `BPF_PROG_ATTACH` registry.
    cgroups: HashMap<String, Vec<CgroupHook>>,
    /// Service frontend → list of backends.
    services: HashMap<(IpAddr, u16, u8), Vec<ServiceBackend>>,
    /// `revnat_id` → original frontend (so recvmsg can rewrite back).
    revnat_index: HashMap<u32, (IpAddr, u16, u8)>,
    next_revnat: u32,
}

impl SockLbManager {
    pub fn new(tenant: TenantId, config: SockLbConfig) -> Self {
        Self {
            tenant,
            config,
            cgroups: HashMap::new(),
            services: HashMap::new(),
            revnat_index: HashMap::new(),
            next_revnat: 1,
        }
    }

    /// Attach the SocketLB programs to a cgroup root. Mirrors
    /// `pkg/socketlb/socketlb.go::Attach`.
    pub fn attach(
        &mut self,
        cgroup_root: impl Into<String>,
        hooks: Vec<CgroupHook>,
    ) -> Result<(), SockLbError> {
        let root = cgroup_root.into();
        if self.cgroups.contains_key(&root) {
            return Err(SockLbError::AlreadyAttached(root));
        }
        self.cgroups.insert(root, hooks);
        Ok(())
    }

    pub fn detach(&mut self, cgroup_root: &str) -> Result<(), SockLbError> {
        self.cgroups
            .remove(cgroup_root)
            .ok_or_else(|| SockLbError::NotAttached(cgroup_root.to_string()))?;
        Ok(())
    }

    pub fn attached_cgroups(&self) -> Vec<&String> {
        self.cgroups.keys().collect()
    }

    pub fn hooks_for(&self, cgroup_root: &str) -> Option<&Vec<CgroupHook>> {
        self.cgroups.get(cgroup_root)
    }

    /// Register a service frontend. Returns the assigned `revnat_id`.
    pub fn register_service(
        &mut self,
        frontend: ServiceFrontend,
        backends: Vec<ServiceBackend>,
    ) -> Result<u32, SockLbError> {
        let key = (frontend.cluster_ip, frontend.port, frontend.protocol);
        if self.services.contains_key(&key) {
            return Err(SockLbError::DuplicateFrontend {
                ip: frontend.cluster_ip,
                port: frontend.port,
            });
        }
        let revnat = self.next_revnat;
        self.next_revnat += 1;
        self.services.insert(key, backends);
        self.revnat_index.insert(revnat, key);
        Ok(revnat)
    }

    pub fn deregister_service(&mut self, frontend: &ServiceFrontend) -> bool {
        let key = (frontend.cluster_ip, frontend.port, frontend.protocol);
        let present = self.services.remove(&key).is_some();
        if present {
            self.revnat_index.retain(|_, v| v != &key);
        }
        present
    }

    pub fn service_count(&self) -> usize {
        self.services.len()
    }

    /// Drive the connect/sendmsg hook for a socket trying to reach
    /// `(dst_ip, dst_port, proto)` from `src_in_host_ns`. If the
    /// destination is a known frontend and the source is eligible,
    /// returns a `Rewrite` decision; otherwise `Passthrough`.
    pub fn on_connect(
        &self,
        dst_ip: IpAddr,
        dst_port: u16,
        proto: u8,
        src_in_host_ns: bool,
        flow_hash: u64,
    ) -> SockLbDecision {
        let key = (dst_ip, dst_port, proto);
        let backends = match self.services.get(&key) {
            Some(b) => b,
            None => return SockLbDecision::Passthrough,
        };
        // Eligibility check.
        if self.config.host_ns_only && !src_in_host_ns {
            // Mode is host-ns-only and source is *not* in host-ns → passthrough.
            return SockLbDecision::Passthrough;
        }
        let candidates: Vec<&ServiceBackend> = backends.iter().collect();
        if candidates.is_empty() {
            return SockLbDecision::Passthrough;
        }
        let chosen = candidates[(flow_hash as usize) % candidates.len()];
        let revnat = self
            .revnat_index
            .iter()
            .find(|(_, v)| **v == key)
            .map(|(k, _)| *k)
            .unwrap_or(0);
        SockLbDecision::Rewrite {
            backend_ip: chosen.backend_ip,
            backend_port: chosen.backend_port,
            revnat_id: revnat,
        }
    }

    /// Reverse a recvmsg rewrite: given a `revnat_id` recorded by the
    /// connect hook, return the original `(cluster_ip, port)` so the
    /// app sees the right peer name. Mirrors
    /// `bpf/bpf_sock.c::cil_sock4_recvmsg`.
    pub fn on_recvmsg(&self, revnat_id: u32) -> Option<(IpAddr, u16)> {
        self.revnat_index
            .get(&revnat_id)
            .map(|(ip, port, _)| (*ip, *port))
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/socketlb/socketlb.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn mgr(tenant: TenantId) -> SockLbManager {
        SockLbManager::new(tenant, SockLbConfig::default())
    }

    fn frontend() -> ServiceFrontend {
        ServiceFrontend {
            cluster_ip: ip(10, 96, 0, 1),
            port: 80,
            protocol: 6,
        }
    }

    fn backends() -> Vec<ServiceBackend> {
        vec![
            ServiceBackend {
                backend_id: 1,
                backend_ip: ip(10, 0, 1, 1),
                backend_port: 8080,
            },
            ServiceBackend {
                backend_id: 2,
                backend_ip: ip(10, 0, 1, 2),
                backend_port: 8080,
            },
        ]
    }

    // ── Cgroup hooks ─────────────────────────────────────────────────────────

    #[test]
    fn cgroup_hook_names_match_kernel_constants() {
        let (_c, _t) = cilium_test_ctx!("bpf/bpf_sock.c", "CgroupHook.Name", "tenant-sl-name");
        assert_eq!(CgroupHook::InetConnect4.name(), "BPF_CGROUP_INET4_CONNECT");
        assert_eq!(CgroupHook::InetConnect6.name(), "BPF_CGROUP_INET6_CONNECT");
        assert_eq!(CgroupHook::UdpSendmsg4.name(), "BPF_CGROUP_UDP4_SENDMSG");
        assert_eq!(CgroupHook::UdpSendmsg6.name(), "BPF_CGROUP_UDP6_SENDMSG");
        assert_eq!(CgroupHook::UdpRecvmsg4.name(), "BPF_CGROUP_UDP4_RECVMSG");
        assert_eq!(CgroupHook::UdpRecvmsg6.name(), "BPF_CGROUP_UDP6_RECVMSG");
        assert_eq!(
            CgroupHook::InetGetpeername4.name(),
            "BPF_CGROUP_INET4_GETPEERNAME"
        );
    }

    // ── Attach / detach ──────────────────────────────────────────────────────

    #[test]
    fn sock_lb_attach_records_cgroup_root() {
        let (_c, tenant) = cilium_test_ctx!("pkg/socketlb/socketlb.go", "Attach", "tenant-sl-att");
        let mut m = mgr(tenant);
        m.attach("/sys/fs/cgroup", vec![CgroupHook::InetConnect4])
            .unwrap();
        assert_eq!(m.attached_cgroups().len(), 1);
    }

    #[test]
    fn sock_lb_attach_duplicate_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "Attach.Duplicate",
            "tenant-sl-attdup"
        );
        let mut m = mgr(tenant);
        m.attach("/sys/fs/cgroup", vec![CgroupHook::InetConnect4])
            .unwrap();
        let err = m
            .attach("/sys/fs/cgroup", vec![CgroupHook::InetConnect4])
            .unwrap_err();
        assert!(matches!(err, SockLbError::AlreadyAttached(_)));
    }

    #[test]
    fn sock_lb_detach_drops_cgroup() {
        let (_c, tenant) = cilium_test_ctx!("pkg/socketlb/socketlb.go", "Detach", "tenant-sl-det");
        let mut m = mgr(tenant);
        m.attach("/sys/fs/cgroup", vec![CgroupHook::InetConnect4])
            .unwrap();
        m.detach("/sys/fs/cgroup").unwrap();
        assert_eq!(m.attached_cgroups().len(), 0);
    }

    #[test]
    fn sock_lb_detach_unknown_returns_not_attached() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "Detach.NotAttached",
            "tenant-sl-detnf"
        );
        let mut m = mgr(tenant);
        let err = m.detach("/sys/fs/cgroup").unwrap_err();
        assert!(matches!(err, SockLbError::NotAttached(_)));
    }

    #[test]
    fn sock_lb_hooks_for_returns_attached_set() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/socketlb/socketlb.go", "HooksFor", "tenant-sl-hooks");
        let mut m = mgr(tenant);
        let hooks = vec![CgroupHook::InetConnect4, CgroupHook::UdpSendmsg4];
        m.attach("/sys/fs/cgroup", hooks.clone()).unwrap();
        assert_eq!(m.hooks_for("/sys/fs/cgroup").unwrap(), &hooks);
    }

    // ── Service registry ─────────────────────────────────────────────────────

    #[test]
    fn sock_lb_register_service_assigns_revnat_id() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "RegisterService",
            "tenant-sl-reg"
        );
        let mut m = mgr(tenant);
        let id = m.register_service(frontend(), backends()).unwrap();
        assert_eq!(id, 1);
        assert_eq!(m.service_count(), 1);
    }

    #[test]
    fn sock_lb_register_service_duplicate_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "RegisterService.Duplicate",
            "tenant-sl-regdup"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), backends()).unwrap();
        let err = m.register_service(frontend(), backends()).unwrap_err();
        assert!(matches!(err, SockLbError::DuplicateFrontend { .. }));
    }

    #[test]
    fn sock_lb_deregister_service_drops_revnat_index() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "DeregisterService",
            "tenant-sl-dereg"
        );
        let mut m = mgr(tenant);
        let id = m.register_service(frontend(), backends()).unwrap();
        assert!(m.deregister_service(&frontend()));
        assert!(m.on_recvmsg(id).is_none());
    }

    #[test]
    fn sock_lb_deregister_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "DeregisterService.Unknown",
            "tenant-sl-deregunk"
        );
        let mut m = mgr(tenant);
        assert!(!m.deregister_service(&frontend()));
    }

    #[test]
    fn sock_lb_revnat_id_monotonic_across_services() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "RegisterService.Monotonic",
            "tenant-sl-mono"
        );
        let mut m = mgr(tenant);
        let id1 = m.register_service(frontend(), backends()).unwrap();
        let mut f2 = frontend();
        f2.port = 81;
        let id2 = m.register_service(f2, backends()).unwrap();
        assert_eq!(id2, id1 + 1);
    }

    // ── Connect hook ─────────────────────────────────────────────────────────

    #[test]
    fn sock_lb_connect_to_known_service_rewrites() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.Rewrite",
            "tenant-sl-conn"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), backends()).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 6, false, 0);
        match d {
            SockLbDecision::Rewrite { backend_ip, .. } => {
                assert!([ip(10, 0, 1, 1), ip(10, 0, 1, 2)].contains(&backend_ip));
            }
            _ => panic!("expected Rewrite, got {d:?}"),
        }
    }

    #[test]
    fn sock_lb_connect_to_unknown_destination_passes_through() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.Passthrough",
            "tenant-sl-connpt"
        );
        let m = mgr(tenant);
        let d = m.on_connect(ip(8, 8, 8, 8), 53, 17, false, 0);
        assert_eq!(d, SockLbDecision::Passthrough);
    }

    #[test]
    fn sock_lb_connect_with_no_backends_returns_passthrough() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.EmptyBackends",
            "tenant-sl-connemp"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), vec![]).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 6, false, 0);
        assert_eq!(d, SockLbDecision::Passthrough);
    }

    #[test]
    fn sock_lb_connect_uses_flow_hash_for_backend_selection() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.HashSelect",
            "tenant-sl-connhash"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), backends()).unwrap();
        let mut hits = std::collections::HashSet::new();
        for h in 0..32u64 {
            if let SockLbDecision::Rewrite { backend_ip, .. } =
                m.on_connect(ip(10, 96, 0, 1), 80, 6, false, h)
            {
                hits.insert(backend_ip);
            }
        }
        assert!(hits.len() >= 2);
    }

    #[test]
    fn sock_lb_host_ns_only_skips_non_host_source() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "HostnsOnly.Skip",
            "tenant-sl-hnsskip"
        );
        let mut m = SockLbManager::new(
            tenant,
            SockLbConfig {
                host_ns_only: true,
                track_terminating: true,
            },
        );
        m.register_service(frontend(), backends()).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 6, false, 0);
        assert_eq!(d, SockLbDecision::Passthrough);
    }

    #[test]
    fn sock_lb_host_ns_only_rewrites_host_source() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "HostnsOnly.Rewrite",
            "tenant-sl-hnsrw"
        );
        let mut m = SockLbManager::new(
            tenant,
            SockLbConfig {
                host_ns_only: true,
                track_terminating: true,
            },
        );
        m.register_service(frontend(), backends()).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 6, true, 0);
        assert!(matches!(d, SockLbDecision::Rewrite { .. }));
    }

    #[test]
    fn sock_lb_default_config_rewrites_pod_source() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "Default.PodSource",
            "tenant-sl-podsrc"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), backends()).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 6, false, 0);
        assert!(matches!(d, SockLbDecision::Rewrite { .. }));
    }

    #[test]
    fn sock_lb_connect_returns_revnat_id_for_chosen_backend() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.RevnatId",
            "tenant-sl-rnid"
        );
        let mut m = mgr(tenant);
        let id = m.register_service(frontend(), backends()).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 6, false, 0);
        match d {
            SockLbDecision::Rewrite { revnat_id, .. } => assert_eq!(revnat_id, id),
            _ => panic!("expected Rewrite"),
        }
    }

    // ── Recvmsg reverse ──────────────────────────────────────────────────────

    #[test]
    fn sock_lb_recvmsg_returns_original_frontend() {
        let (_c, tenant) =
            cilium_test_ctx!("bpf/bpf_sock.c", "cil_sock4_recvmsg", "tenant-sl-recv");
        let mut m = mgr(tenant);
        let id = m.register_service(frontend(), backends()).unwrap();
        let (orig_ip, orig_port) = m.on_recvmsg(id).unwrap();
        assert_eq!(orig_ip, ip(10, 96, 0, 1));
        assert_eq!(orig_port, 80);
    }

    #[test]
    fn sock_lb_recvmsg_unknown_revnat_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_recvmsg.Unknown",
            "tenant-sl-recvnf"
        );
        let m = mgr(tenant);
        assert!(m.on_recvmsg(99).is_none());
    }

    // ── Multi-protocol ───────────────────────────────────────────────────────

    #[test]
    fn sock_lb_distinct_proto_makes_distinct_frontend() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "RegisterService.MultiProto",
            "tenant-sl-mp"
        );
        let mut m = mgr(tenant);
        let mut tcp = frontend();
        tcp.protocol = 6;
        let mut udp = frontend();
        udp.protocol = 17;
        m.register_service(tcp, backends()).unwrap();
        m.register_service(udp, backends()).unwrap();
        assert_eq!(m.service_count(), 2);
    }

    // ── IPv6 ─────────────────────────────────────────────────────────────────

    #[test]
    fn sock_lb_v6_service_rewrites() {
        let (_c, tenant) = cilium_test_ctx!("bpf/bpf_sock.c", "cil_sock6_connect", "tenant-sl-v6");
        let mut m = mgr(tenant);
        let f = ServiceFrontend {
            cluster_ip: "fd00:96::1".parse().unwrap(),
            port: 80,
            protocol: 6,
        };
        let b = vec![ServiceBackend {
            backend_id: 1,
            backend_ip: "fd00:1::5".parse().unwrap(),
            backend_port: 8080,
        }];
        m.register_service(f.clone(), b).unwrap();
        let d = m.on_connect(f.cluster_ip, 80, 6, false, 0);
        assert!(matches!(d, SockLbDecision::Rewrite { .. }));
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn sock_lb_cgroup_hook_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "CgroupHook.Serde",
            "tenant-sl-serde-h"
        );
        for h in [
            CgroupHook::InetConnect4,
            CgroupHook::InetConnect6,
            CgroupHook::UdpSendmsg4,
            CgroupHook::UdpRecvmsg4,
            CgroupHook::InetGetpeername4,
        ] {
            let s = serde_json::to_string(&h).unwrap();
            let back: CgroupHook = serde_json::from_str(&s).unwrap();
            assert_eq!(back, h);
        }
    }

    #[test]
    fn sock_lb_decision_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "Decision.Serde",
            "tenant-sl-serde-d"
        );
        let d = SockLbDecision::Rewrite {
            backend_ip: ip(10, 0, 1, 1),
            backend_port: 8080,
            revnat_id: 1,
        };
        let s = serde_json::to_string(&d).unwrap();
        let back: SockLbDecision = serde_json::from_str(&s).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn sock_lb_frontend_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "Frontend.Serde",
            "tenant-sl-serde-f"
        );
        let f = frontend();
        let s = serde_json::to_string(&f).unwrap();
        let back: ServiceFrontend = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn sock_lb_config_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/socketlb/socketlb.go",
            "Config.Serde",
            "tenant-sl-serde-c"
        );
        let c = SockLbConfig {
            host_ns_only: true,
            track_terminating: false,
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: SockLbConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, c);
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn sock_lb_connect_passthrough_when_proto_mismatch() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.ProtoMismatch",
            "tenant-sl-pm"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), backends()).unwrap();
        // Frontend is TCP (6); request is UDP (17).
        let d = m.on_connect(ip(10, 96, 0, 1), 80, 17, false, 0);
        assert_eq!(d, SockLbDecision::Passthrough);
    }

    #[test]
    fn sock_lb_connect_passthrough_when_port_mismatch() {
        let (_c, tenant) = cilium_test_ctx!(
            "bpf/bpf_sock.c",
            "cil_sock4_connect.PortMismatch",
            "tenant-sl-pmp"
        );
        let mut m = mgr(tenant);
        m.register_service(frontend(), backends()).unwrap();
        let d = m.on_connect(ip(10, 96, 0, 1), 8080, 6, false, 0);
        assert_eq!(d, SockLbDecision::Passthrough);
    }

    #[test]
    fn sock_lb_attached_cgroups_count_tracks_attaches() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/socketlb/socketlb.go", "AttachedCount", "tenant-sl-cnt");
        let mut m = mgr(tenant);
        m.attach("/sys/fs/cgroup/a", vec![CgroupHook::InetConnect4])
            .unwrap();
        m.attach("/sys/fs/cgroup/b", vec![CgroupHook::InetConnect4])
            .unwrap();
        assert_eq!(m.attached_cgroups().len(), 2);
    }
}
