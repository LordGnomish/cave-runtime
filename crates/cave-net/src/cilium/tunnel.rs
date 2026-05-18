// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tunneling — VXLAN / Geneve / native-routing encap modes.
//!
//! Mirrors `pkg/datapath/tunnel/tunnel.go` (the per-mode tunnel manager)
//! and `pkg/maps/tunnel/tunnel.go` (the BPF map shape that maps remote
//! pod CIDRs to the tunnel endpoint of the owning node).
//!
//! Semantics (faithful to upstream):
//!
//! * [`TunnelMode::Vxlan`] (default) — encap with VXLAN over UDP/8472,
//!   VNI 0 by default. Adds a 50-byte overhead (14 outer L2 + 20 outer
//!   IPv4 + 8 outer UDP + 8 VXLAN header).
//! * [`TunnelMode::Geneve`] — encap with Geneve over UDP/6081. Same
//!   50-byte minimum overhead; options can grow it further.
//! * [`TunnelMode::Disabled`] (native routing) — no encap; relies on
//!   external L3 reachability for the pod-CIDR. Requires a
//!   `native_routing_cidr` config so cilium can decide which packets
//!   to leave untouched.
//! * Per remote node, cilium maintains a `TunnelEndpoint` (node IP +
//!   pod CIDR) so the egress program can `lookup_encap(dst_pod_ip)`
//!   to find the right tunnel destination.
//! * Within `native_routing_cidr`, packets are forwarded plain (no
//!   encap) regardless of tunnel mode.

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TunnelMode {
    Vxlan,
    Geneve,
    /// No encapsulation — relies on native L3 routing.
    Disabled,
}

impl TunnelMode {
    /// Default UDP port for the tunnel mode (mirrors
    /// `pkg/option/config.go::TunnelPort`).
    pub fn default_port(self) -> Option<u16> {
        match self {
            TunnelMode::Vxlan => Some(8472),
            TunnelMode::Geneve => Some(6081),
            TunnelMode::Disabled => None,
        }
    }

    /// Per-packet overhead the encap adds (bytes). Mirrors
    /// `pkg/mtu/mtu.go::EncapOverhead*`.
    pub fn encap_overhead(self) -> usize {
        match self {
            TunnelMode::Vxlan => 50,
            TunnelMode::Geneve => 50,
            TunnelMode::Disabled => 0,
        }
    }

    /// In-kernel interface name produced by the agent.
    pub fn interface_name(self) -> Option<&'static str> {
        match self {
            TunnelMode::Vxlan => Some("cilium_vxlan"),
            TunnelMode::Geneve => Some("cilium_geneve"),
            TunnelMode::Disabled => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelEndpoint {
    pub node_name: String,
    pub node_ip: IpAddr,
    pub pod_cidr: String,
    /// VNI (VXLAN) or VNI-equivalent (Geneve "vni" option).
    pub vni: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TunnelError {
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("Disabled mode requires a native_routing_cidr")]
    NativeRoutingCidrRequired,
    #[error("node `{0}` not found")]
    NodeNotFound(String),
    #[error("tenant {tenant} cannot mutate tunnel manager owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncapDecision {
    /// Send the packet through the tunnel to the recorded endpoint.
    Encap { node_ip: IpAddr, vni: u32 },
    /// Forward natively (no encap) — destination is in
    /// `native_routing_cidr`.
    Native,
    /// Destination is unknown — drop or upcall.
    Unknown,
}

#[derive(Debug)]
pub struct TunnelManager {
    pub tenant: TenantId,
    pub mode: TunnelMode,
    pub native_routing_cidr: Option<String>,
    endpoints: HashMap<String, TunnelEndpoint>,
}

impl TunnelManager {
    pub fn new(tenant: TenantId, mode: TunnelMode, native_routing_cidr: Option<String>) -> Result<Self, TunnelError> {
        if matches!(mode, TunnelMode::Disabled) && native_routing_cidr.is_none() {
            return Err(TunnelError::NativeRoutingCidrRequired);
        }
        if let Some(c) = &native_routing_cidr {
            IpNet::from_str(c).map_err(|_| TunnelError::BadCidr(c.clone()))?;
        }
        Ok(Self { tenant, mode, native_routing_cidr, endpoints: HashMap::new() })
    }

    pub fn upsert_endpoint(&mut self, ep: TunnelEndpoint) -> Result<(), TunnelError> {
        IpNet::from_str(&ep.pod_cidr).map_err(|_| TunnelError::BadCidr(ep.pod_cidr.clone()))?;
        self.endpoints.insert(ep.node_name.clone(), ep);
        Ok(())
    }

    pub fn remove_endpoint(&mut self, node: &str) -> Result<(), TunnelError> {
        self.endpoints.remove(node).ok_or_else(|| TunnelError::NodeNotFound(node.to_string()))?;
        Ok(())
    }

    pub fn endpoint_count(&self) -> usize {
        self.endpoints.len()
    }

    pub fn endpoint(&self, node: &str) -> Option<&TunnelEndpoint> {
        self.endpoints.get(node)
    }

    /// Resolve the encap decision for a packet heading to `dst_ip`.
    /// Mirrors `bpf/lib/tunnel.h::tunnel_lookup`.
    pub fn lookup_encap(&self, dst_ip: IpAddr) -> Result<EncapDecision, TunnelError> {
        if let Some(c) = &self.native_routing_cidr {
            let net = IpNet::from_str(c).map_err(|_| TunnelError::BadCidr(c.clone()))?;
            if net.contains(&dst_ip) {
                return Ok(EncapDecision::Native);
            }
        }
        if matches!(self.mode, TunnelMode::Disabled) {
            // No tunnel and not in native CIDR → unknown.
            return Ok(EncapDecision::Unknown);
        }
        for ep in self.endpoints.values() {
            let net = IpNet::from_str(&ep.pod_cidr).map_err(|_| TunnelError::BadCidr(ep.pod_cidr.clone()))?;
            if net.contains(&dst_ip) {
                return Ok(EncapDecision::Encap { node_ip: ep.node_ip, vni: ep.vni });
            }
        }
        Ok(EncapDecision::Unknown)
    }

    /// Compute MTU after encap overhead. Mirrors
    /// `pkg/mtu/mtu.go::Configuration.GetDeviceMTU`.
    pub fn effective_mtu(&self, link_mtu: usize) -> usize {
        link_mtu.saturating_sub(self.mode.encap_overhead())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/datapath/tunnel/tunnel.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn vxlan_mgr(tenant: TenantId) -> TunnelManager {
        TunnelManager::new(tenant, TunnelMode::Vxlan, None).unwrap()
    }

    fn make_endpoint(node: &str, ip4: (u8, u8, u8, u8), cidr: &str, vni: u32) -> TunnelEndpoint {
        TunnelEndpoint {
            node_name: node.into(),
            node_ip: ip(ip4.0, ip4.1, ip4.2, ip4.3),
            pod_cidr: cidr.into(),
            vni,
        }
    }

    // ── Mode helpers ─────────────────────────────────────────────────────────

    #[test]
    fn tunnel_default_port_per_mode() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "TunnelPort", "tenant-tun-port");
        assert_eq!(TunnelMode::Vxlan.default_port(), Some(8472));
        assert_eq!(TunnelMode::Geneve.default_port(), Some(6081));
        assert_eq!(TunnelMode::Disabled.default_port(), None);
    }

    #[test]
    fn tunnel_encap_overhead_per_mode() {
        let (_c, _t) = cilium_test_ctx!("pkg/mtu/mtu.go", "EncapOverhead", "tenant-tun-ovr");
        assert_eq!(TunnelMode::Vxlan.encap_overhead(), 50);
        assert_eq!(TunnelMode::Geneve.encap_overhead(), 50);
        assert_eq!(TunnelMode::Disabled.encap_overhead(), 0);
    }

    #[test]
    fn tunnel_interface_name_per_mode() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.IfaceName", "tenant-tun-if");
        assert_eq!(TunnelMode::Vxlan.interface_name(), Some("cilium_vxlan"));
        assert_eq!(TunnelMode::Geneve.interface_name(), Some("cilium_geneve"));
        assert_eq!(TunnelMode::Disabled.interface_name(), None);
    }

    // ── Construction ─────────────────────────────────────────────────────────

    #[test]
    fn tunnel_disabled_mode_without_native_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.New.DisabledRequiresNative", "tenant-tun-disreq");
        let err = TunnelManager::new(tenant, TunnelMode::Disabled, None).unwrap_err();
        assert_eq!(err, TunnelError::NativeRoutingCidrRequired);
    }

    #[test]
    fn tunnel_disabled_mode_with_native_cidr_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.New.Disabled", "tenant-tun-disok");
        let m = TunnelManager::new(tenant, TunnelMode::Disabled, Some("10.0.0.0/16".into())).unwrap();
        assert_eq!(m.mode, TunnelMode::Disabled);
    }

    #[test]
    fn tunnel_bad_native_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.New.BadCidr", "tenant-tun-badcidr");
        let err = TunnelManager::new(tenant, TunnelMode::Vxlan, Some("not-a-cidr".into())).unwrap_err();
        assert_eq!(err, TunnelError::BadCidr("not-a-cidr".into()));
    }

    #[test]
    fn tunnel_vxlan_default_no_native_cidr() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.New.Vxlan", "tenant-tun-vxd");
        let m = TunnelManager::new(tenant, TunnelMode::Vxlan, None).unwrap();
        assert!(m.native_routing_cidr.is_none());
    }

    // ── Endpoint lifecycle ───────────────────────────────────────────────────

    #[test]
    fn tunnel_upsert_endpoint_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.UpsertEndpoint", "tenant-tun-up");
        let mut m = vxlan_mgr(tenant);
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        assert_eq!(m.endpoint_count(), 1);
    }

    #[test]
    fn tunnel_upsert_endpoint_with_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.UpsertEndpoint.BadCidr", "tenant-tun-upbad");
        let mut m = vxlan_mgr(tenant);
        let mut bad = make_endpoint("node-a", (10, 0, 0, 1), "nope", 0);
        bad.pod_cidr = "nope".into();
        assert!(m.upsert_endpoint(bad).is_err());
    }

    #[test]
    fn tunnel_upsert_endpoint_replaces_in_place() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.UpsertEndpoint.Replace", "tenant-tun-uprep");
        let mut m = vxlan_mgr(tenant);
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 99), "10.244.1.0/24", 0)).unwrap();
        assert_eq!(m.endpoint_count(), 1);
        assert_eq!(m.endpoint("node-a").unwrap().node_ip, ip(10, 0, 0, 99));
    }

    #[test]
    fn tunnel_remove_endpoint_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.RemoveEndpoint", "tenant-tun-rm");
        let mut m = vxlan_mgr(tenant);
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        m.remove_endpoint("node-a").unwrap();
        assert_eq!(m.endpoint_count(), 0);
    }

    #[test]
    fn tunnel_remove_unknown_endpoint_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.RemoveEndpoint.NotFound", "tenant-tun-rmnf");
        let mut m = vxlan_mgr(tenant);
        let err = m.remove_endpoint("ghost").unwrap_err();
        assert!(matches!(err, TunnelError::NodeNotFound(_)));
    }

    #[test]
    fn tunnel_endpoint_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.Endpoint.NotFound", "tenant-tun-lknf");
        let m = vxlan_mgr(tenant);
        assert!(m.endpoint("ghost").is_none());
    }

    #[test]
    fn tunnel_endpoint_count_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.EndpointCount", "tenant-tun-cnt");
        let mut m = vxlan_mgr(tenant);
        for i in 0..5u8 {
            m.upsert_endpoint(make_endpoint(&format!("node-{i}"), (10, 0, 0, i + 1), &format!("10.244.{i}.0/24"), 0)).unwrap();
        }
        assert_eq!(m.endpoint_count(), 5);
    }

    // ── Encap decision ───────────────────────────────────────────────────────

    #[test]
    fn tunnel_lookup_encap_routes_to_owning_node() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup", "tenant-tun-lk");
        let mut m = vxlan_mgr(tenant);
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        m.upsert_endpoint(make_endpoint("node-b", (10, 0, 0, 2), "10.244.2.0/24", 0)).unwrap();
        let d = m.lookup_encap(ip(10, 244, 2, 5)).unwrap();
        assert_eq!(d, EncapDecision::Encap { node_ip: ip(10, 0, 0, 2), vni: 0 });
    }

    #[test]
    fn tunnel_lookup_encap_unknown_destination_returns_unknown() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup.Unknown", "tenant-tun-lkunk");
        let mut m = vxlan_mgr(tenant);
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        let d = m.lookup_encap(ip(8, 8, 8, 8)).unwrap();
        assert_eq!(d, EncapDecision::Unknown);
    }

    #[test]
    fn tunnel_native_routing_cidr_skips_encap() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup.Native", "tenant-tun-natv");
        let mut m = TunnelManager::new(tenant, TunnelMode::Vxlan, Some("172.16.0.0/12".into())).unwrap();
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        let d = m.lookup_encap(ip(172, 16, 5, 1)).unwrap();
        assert_eq!(d, EncapDecision::Native);
    }

    #[test]
    fn tunnel_native_routing_cidr_overrides_endpoint_match() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup.NativeOverEndpoint", "tenant-tun-natov");
        let mut m = TunnelManager::new(tenant, TunnelMode::Vxlan, Some("10.244.0.0/16".into())).unwrap();
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 0)).unwrap();
        let d = m.lookup_encap(ip(10, 244, 1, 5)).unwrap();
        assert_eq!(d, EncapDecision::Native);
    }

    #[test]
    fn tunnel_disabled_mode_unknown_destination_unknown() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup.DisabledUnknown", "tenant-tun-disunk");
        let m = TunnelManager::new(tenant, TunnelMode::Disabled, Some("10.0.0.0/8".into())).unwrap();
        let d = m.lookup_encap(ip(8, 8, 8, 8)).unwrap();
        assert_eq!(d, EncapDecision::Unknown);
    }

    #[test]
    fn tunnel_disabled_mode_native_routing_cidr_returns_native() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup.DisabledNative", "tenant-tun-disnat");
        let m = TunnelManager::new(tenant, TunnelMode::Disabled, Some("10.0.0.0/8".into())).unwrap();
        let d = m.lookup_encap(ip(10, 1, 2, 3)).unwrap();
        assert_eq!(d, EncapDecision::Native);
    }

    #[test]
    fn tunnel_geneve_mode_encaps_traffic() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.Geneve", "tenant-tun-gn");
        let mut m = TunnelManager::new(tenant, TunnelMode::Geneve, None).unwrap();
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 7)).unwrap();
        let d = m.lookup_encap(ip(10, 244, 1, 5)).unwrap();
        assert_eq!(d, EncapDecision::Encap { node_ip: ip(10, 0, 0, 1), vni: 7 });
    }

    // ── MTU ──────────────────────────────────────────────────────────────────

    #[test]
    fn tunnel_effective_mtu_subtracts_overhead() {
        let (_c, tenant) = cilium_test_ctx!("pkg/mtu/mtu.go", "EffectiveMTU", "tenant-tun-mtu");
        let m = vxlan_mgr(tenant);
        assert_eq!(m.effective_mtu(1500), 1450);
    }

    #[test]
    fn tunnel_effective_mtu_disabled_no_change() {
        let (_c, tenant) = cilium_test_ctx!("pkg/mtu/mtu.go", "EffectiveMTU.Disabled", "tenant-tun-mtudis");
        let m = TunnelManager::new(tenant, TunnelMode::Disabled, Some("10.0.0.0/8".into())).unwrap();
        assert_eq!(m.effective_mtu(1500), 1500);
    }

    #[test]
    fn tunnel_effective_mtu_underflow_clamps_to_zero() {
        let (_c, tenant) = cilium_test_ctx!("pkg/mtu/mtu.go", "EffectiveMTU.Underflow", "tenant-tun-mtuun");
        let m = vxlan_mgr(tenant);
        assert_eq!(m.effective_mtu(40), 0);
    }

    // ── Dual stack ───────────────────────────────────────────────────────────

    #[test]
    fn tunnel_endpoint_v6_pod_cidr() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Endpoint.IPv6", "tenant-tun-v6");
        let mut m = vxlan_mgr(tenant);
        let ep = TunnelEndpoint {
            node_name: "node-a".into(),
            node_ip: "2001:db8::1".parse().unwrap(),
            pod_cidr: "fd00:1::/64".into(),
            vni: 0,
        };
        m.upsert_endpoint(ep).unwrap();
        let dst: IpAddr = "fd00:1::5".parse().unwrap();
        let d = m.lookup_encap(dst).unwrap();
        assert!(matches!(d, EncapDecision::Encap { .. }));
    }

    // ── VNI ─────────────────────────────────────────────────────────────────

    #[test]
    fn tunnel_vni_carries_through_to_encap_decision() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/tunnel.h", "tunnel_lookup.VNI", "tenant-tun-vni");
        let mut m = vxlan_mgr(tenant);
        m.upsert_endpoint(make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 42)).unwrap();
        let d = m.lookup_encap(ip(10, 244, 1, 5)).unwrap();
        match d {
            EncapDecision::Encap { vni, .. } => assert_eq!(vni, 42),
            other => panic!("expected Encap, got {other:?}"),
        }
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn tunnel_mode_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/option/config.go", "TunnelMode.Serde", "tenant-tun-serde-mode");
        for m in [TunnelMode::Vxlan, TunnelMode::Geneve, TunnelMode::Disabled] {
            let s = serde_json::to_string(&m).unwrap();
            let back: TunnelMode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn tunnel_endpoint_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "TunnelEndpoint.Serde", "tenant-tun-serde-ep");
        let ep = make_endpoint("node-a", (10, 0, 0, 1), "10.244.1.0/24", 7);
        let json = serde_json::to_string(&ep).unwrap();
        let back: TunnelEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ep);
    }

    #[test]
    fn tunnel_encap_decision_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/tunnel.h", "EncapDecision.Serde", "tenant-tun-serde-d");
        for d in [
            EncapDecision::Encap { node_ip: ip(10, 0, 0, 1), vni: 0 },
            EncapDecision::Native,
            EncapDecision::Unknown,
        ] {
            let s = serde_json::to_string(&d).unwrap();
            let back: EncapDecision = serde_json::from_str(&s).unwrap();
            assert_eq!(back, d);
        }
    }

    // ── Multi-node ────────────────────────────────────────────────────────────

    #[test]
    fn tunnel_multiple_endpoints_distinct_pod_cidrs() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.MultipleEndpoints", "tenant-tun-multi");
        let mut m = vxlan_mgr(tenant);
        for i in 0..5u8 {
            m.upsert_endpoint(make_endpoint(&format!("node-{i}"), (10, 0, 0, i + 1), &format!("10.244.{i}.0/24"), i as u32)).unwrap();
        }
        for i in 0..5u8 {
            let dst = ip(10, 244, i, 5);
            let d = m.lookup_encap(dst).unwrap();
            match d {
                EncapDecision::Encap { node_ip, vni } => {
                    assert_eq!(node_ip, ip(10, 0, 0, i + 1));
                    assert_eq!(vni, i as u32);
                }
                other => panic!("i={i}, got {other:?}"),
            }
        }
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn tunnel_native_routing_cidr_empty_endpoints_unknown() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/tunnel/tunnel.go", "Manager.NoEndpoints", "tenant-tun-noep");
        let m = TunnelManager::new(tenant, TunnelMode::Vxlan, Some("10.0.0.0/16".into())).unwrap();
        // dst not in native CIDR and no endpoints → Unknown.
        let d = m.lookup_encap(ip(192, 168, 1, 1)).unwrap();
        assert_eq!(d, EncapDecision::Unknown);
    }
}
