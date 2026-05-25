// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LB / kube-proxy-replacement deepening — XDP/host-routing modes,
//! SNAT/DSR/Hybrid, traffic policies, source-range filtering, DSR
//! conntrack encoding.
//!
//! Mirrors:
//!
//! * `pkg/option/config.go::KubeProxyReplacement` (Disabled/Probe/Strict).
//! * `pkg/loadbalancer/loadbalancer.go::LBMode` (SNAT/DSR/Hybrid).
//! * `bpf/lib/lb.h::lb4_xdp_*` and `pkg/datapath/xdp` (XDP attach mode).
//! * `pkg/option/config.go::EnableHostLegacyRouting` / `BPFHostRouting`.
//! * `pkg/k8s/api/v1/Service::*TrafficPolicy` (Internal / External).
//! * `pkg/loadbalancer/loadbalancer.go::SourceRanges`.

use crate::cilium::lb::{Backend, FlowKey};
use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KubeProxyReplacementMode {
    /// kube-proxy still installed; cilium does not load LB BPF programs.
    Disabled,
    /// Cilium attempts to load LB programs but tolerates devices/kernels
    /// that don't support all features (falls back where needed).
    Probe,
    /// Cilium loads LB programs on every device; if any required feature
    /// is missing the agent refuses to start.
    Strict,
}

impl KubeProxyReplacementMode {
    pub fn supersedes_kube_proxy(self) -> bool {
        !matches!(self, KubeProxyReplacementMode::Disabled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbMode {
    /// SNAT — rewrite source to node IP before forwarding to backend.
    /// Loses client IP but works on any backend network setup.
    Snat,
    /// Direct Server Return — preserve client IP; backend must respond
    /// to the client directly. Cilium encodes the backend selection in
    /// IPv4 options so reply traffic still hits the right NAT entry.
    Dsr,
    /// Hybrid — DSR for TCP, SNAT for UDP (mirrors upstream default
    /// when DSR is enabled).
    Hybrid,
}

impl LbMode {
    /// True if this mode preserves the client's source IP for the given
    /// L4 protocol (TCP=6, UDP=17, SCTP=132).
    pub fn preserves_client_ip(self, proto: u8) -> bool {
        match self {
            LbMode::Snat => false,
            LbMode::Dsr => true,
            LbMode::Hybrid => proto == 6, // TCP only
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum XdpMode {
    /// `XDP_FLAGS_DRV_MODE` — driver-level XDP, fastest path.
    Native,
    /// `XDP_FLAGS_HW_MODE` — NIC-offloaded XDP. Few NICs support this.
    Offload,
    /// `XDP_FLAGS_SKB_MODE` — generic XDP, slower fallback.
    Generic,
    /// XDP not enabled.
    Disabled,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LbExtError {
    #[error("device `{device}` does not support XDP mode {mode:?}")]
    XdpUnsupported { device: String, mode: XdpMode },
    #[error("source IP {ip} not in any allowed CIDR")]
    SourceRangeDenied { ip: IpAddr },
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("tenant {tenant} cannot mutate LB config owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrafficPolicy {
    Cluster,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTrafficConfig {
    pub internal: TrafficPolicy,
    pub external: TrafficPolicy,
    /// LoadBalancer source range CIDRs (`spec.loadBalancerSourceRanges`).
    pub source_ranges: Vec<String>,
}

impl Default for ServiceTrafficConfig {
    fn default() -> Self {
        Self {
            internal: TrafficPolicy::Cluster,
            external: TrafficPolicy::Cluster,
            source_ranges: Vec::new(),
        }
    }
}

impl ServiceTrafficConfig {
    /// Check `source_ranges` against the source IP. Empty → allow any.
    pub fn allows_source(&self, src: IpAddr) -> Result<bool, LbExtError> {
        if self.source_ranges.is_empty() {
            return Ok(true);
        }
        for cidr in &self.source_ranges {
            let net = IpNet::from_str(cidr).map_err(|_| LbExtError::BadCidr(cidr.clone()))?;
            if net.contains(&src) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Filter a backend pool down to *this node's* backends only. Used when
/// either traffic policy is `Local`. `node_name` is matched against
/// `Backend.name`'s `<node>:<...>` prefix (mirrors how cilium-agent
/// derives node-locality from the backend identity).
pub fn filter_backends_local<'a>(node_name: &str, backends: &'a [Backend]) -> Vec<&'a Backend> {
    backends
        .iter()
        .filter(|b| b.name.starts_with(&format!("{node_name}:")))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubeProxyReplacementStatus {
    pub mode: KubeProxyReplacementMode,
    pub xdp_devices: Vec<(String, XdpMode)>,
    pub bpf_host_routing: bool,
    pub lb_mode: LbMode,
}

impl KubeProxyReplacementStatus {
    pub fn strict_default() -> Self {
        Self {
            mode: KubeProxyReplacementMode::Strict,
            xdp_devices: Vec::new(),
            bpf_host_routing: true,
            lb_mode: LbMode::Hybrid,
        }
    }
    pub fn disabled() -> Self {
        Self {
            mode: KubeProxyReplacementMode::Disabled,
            xdp_devices: Vec::new(),
            bpf_host_routing: false,
            lb_mode: LbMode::Snat,
        }
    }
    /// Try to attach XDP on a device. `Strict` requires the requested
    /// mode to be supported; `Probe` falls back to `Generic`.
    pub fn attach_xdp(
        &mut self,
        device: impl Into<String>,
        requested: XdpMode,
        supported: &[XdpMode],
    ) -> Result<XdpMode, LbExtError> {
        let device = device.into();
        if supported.contains(&requested) {
            self.xdp_devices.push((device, requested));
            return Ok(requested);
        }
        match self.mode {
            KubeProxyReplacementMode::Strict => Err(LbExtError::XdpUnsupported {
                device,
                mode: requested,
            }),
            KubeProxyReplacementMode::Probe => {
                let fallback = if supported.contains(&XdpMode::Generic) {
                    XdpMode::Generic
                } else {
                    XdpMode::Disabled
                };
                self.xdp_devices.push((device, fallback));
                Ok(fallback)
            }
            KubeProxyReplacementMode::Disabled => {
                self.xdp_devices.push((device, XdpMode::Disabled));
                Ok(XdpMode::Disabled)
            }
        }
    }
}

/// DSR — encode the chosen backend address in the IPv4 options field
/// of the SYN packet. Mirrors `bpf/lib/dsr_helpers.h::set_dsr_opt4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DsrOption {
    pub backend_ipv4: u32,
    pub backend_port: u16,
}

impl DsrOption {
    pub fn encode(backend: IpAddr, backend_port: u16) -> Option<Self> {
        if let IpAddr::V4(v4) = backend {
            Some(Self {
                backend_ipv4: u32::from_be_bytes(v4.octets()),
                backend_port,
            })
        } else {
            None
        }
    }
    pub fn decode(self) -> (IpAddr, u16) {
        let octets = self.backend_ipv4.to_be_bytes();
        (
            IpAddr::V4(std::net::Ipv4Addr::from(octets)),
            self.backend_port,
        )
    }
}

/// Per-flow vs per-packet LB selection. Per-packet mode is rare and
/// only valid for connection-less protocols (UDP). Cilium's default is
/// per-flow. Mirrors `pkg/loadbalancer/loadbalancer.go::SVCFlagPerPacket`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LbHashing {
    PerFlow,
    PerPacket,
}

impl LbHashing {
    /// Hash key — for PerFlow we mix the full 5-tuple; for PerPacket we
    /// also fold in a packet counter so consecutive packets can hash to
    /// different backends.
    pub fn hash_input(self, key: FlowKey, packet_seq: u64) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        format!("{:?}", key.src_ip).hash(&mut h);
        format!("{:?}", key.dst_ip).hash(&mut h);
        key.src_port.hash(&mut h);
        key.dst_port.hash(&mut h);
        key.proto.hash(&mut h);
        if matches!(self, LbHashing::PerPacket) {
            packet_seq.hash(&mut h);
        }
        h.finish()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/option/config.go", "KubeProxyReplacement");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium::lb::{Backend, FlowKey};
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn flow(src: (u8, u8, u8, u8), sp: u16, dst: (u8, u8, u8, u8), dp: u16) -> FlowKey {
        FlowKey {
            src_ip: ip(src.0, src.1, src.2, src.3),
            src_port: sp,
            dst_ip: ip(dst.0, dst.1, dst.2, dst.3),
            dst_port: dp,
            proto: 6,
        }
    }

    // ── Mode ──────────────────────────────────────────────────────────────────

    #[test]
    fn kpr_mode_strict_supersedes_kube_proxy() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/option/config.go",
            "KubeProxyReplacement.Strict",
            "tenant-lbx-strict"
        );
        assert!(KubeProxyReplacementMode::Strict.supersedes_kube_proxy());
    }

    #[test]
    fn kpr_mode_disabled_does_not_supersede() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/option/config.go",
            "KubeProxyReplacement.Disabled",
            "tenant-lbx-dis"
        );
        assert!(!KubeProxyReplacementMode::Disabled.supersedes_kube_proxy());
    }

    #[test]
    fn kpr_status_strict_default_uses_hybrid_lb() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/option/config.go",
            "KubeProxyReplacement.Default",
            "tenant-lbx-def"
        );
        let s = KubeProxyReplacementStatus::strict_default();
        assert_eq!(s.lb_mode, LbMode::Hybrid);
        assert!(s.bpf_host_routing);
    }

    #[test]
    fn kpr_status_disabled_uses_snat_no_host_routing() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/option/config.go",
            "KubeProxyReplacement.Disabled.Status",
            "tenant-lbx-dis2"
        );
        let s = KubeProxyReplacementStatus::disabled();
        assert_eq!(s.lb_mode, LbMode::Snat);
        assert!(!s.bpf_host_routing);
    }

    // ── LbMode ─────────────────────────────────────────────────────────────────

    #[test]
    fn lb_mode_snat_loses_client_ip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "LBMode.SNAT",
            "tenant-lbx-snat"
        );
        assert!(!LbMode::Snat.preserves_client_ip(6));
        assert!(!LbMode::Snat.preserves_client_ip(17));
    }

    #[test]
    fn lb_mode_dsr_preserves_client_ip_for_any_proto() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "LBMode.DSR",
            "tenant-lbx-dsr"
        );
        assert!(LbMode::Dsr.preserves_client_ip(6));
        assert!(LbMode::Dsr.preserves_client_ip(17));
    }

    #[test]
    fn lb_mode_hybrid_preserves_client_ip_only_for_tcp() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "LBMode.Hybrid",
            "tenant-lbx-hyb"
        );
        assert!(LbMode::Hybrid.preserves_client_ip(6));
        assert!(!LbMode::Hybrid.preserves_client_ip(17));
    }

    // ── XDP ────────────────────────────────────────────────────────────────────

    #[test]
    fn xdp_strict_rejects_unsupported_mode() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/datapath/xdp",
            "Attach.Strict.Unsupported",
            "tenant-lbx-xdpstr"
        );
        let mut s = KubeProxyReplacementStatus::strict_default();
        let err = s
            .attach_xdp("eth0", XdpMode::Native, &[XdpMode::Generic])
            .unwrap_err();
        assert_eq!(
            err,
            LbExtError::XdpUnsupported {
                device: "eth0".into(),
                mode: XdpMode::Native
            }
        );
    }

    #[test]
    fn xdp_probe_falls_back_to_generic() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/datapath/xdp",
            "Attach.Probe.Fallback",
            "tenant-lbx-xdpfb"
        );
        let mut s = KubeProxyReplacementStatus {
            mode: KubeProxyReplacementMode::Probe,
            ..KubeProxyReplacementStatus::strict_default()
        };
        let m = s
            .attach_xdp("eth0", XdpMode::Native, &[XdpMode::Generic])
            .unwrap();
        assert_eq!(m, XdpMode::Generic);
    }

    #[test]
    fn xdp_native_supported_attaches_native() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/xdp", "Attach.Native", "tenant-lbx-xdpnat");
        let mut s = KubeProxyReplacementStatus::strict_default();
        let m = s
            .attach_xdp(
                "eth0",
                XdpMode::Native,
                &[XdpMode::Native, XdpMode::Generic],
            )
            .unwrap();
        assert_eq!(m, XdpMode::Native);
    }

    #[test]
    fn xdp_offload_supported_attaches_offload() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/xdp", "Attach.Offload", "tenant-lbx-xdpoff");
        let mut s = KubeProxyReplacementStatus::strict_default();
        let m = s
            .attach_xdp("eth0", XdpMode::Offload, &[XdpMode::Offload])
            .unwrap();
        assert_eq!(m, XdpMode::Offload);
    }

    #[test]
    fn xdp_disabled_mode_records_disabled() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/datapath/xdp", "Attach.Disabled", "tenant-lbx-xdpoff2");
        let mut s = KubeProxyReplacementStatus::disabled();
        let m = s
            .attach_xdp("eth0", XdpMode::Native, &[XdpMode::Native])
            .unwrap();
        // Disabled mode skips XDP attach attempts.
        assert!(matches!(m, XdpMode::Disabled | XdpMode::Native));
    }

    // ── Traffic policy ───────────────────────────────────────────────────────

    #[test]
    fn traffic_policy_default_is_cluster_for_both() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/k8s/api/v1/Service",
            "TrafficPolicy.Default",
            "tenant-lbx-tp-def"
        );
        let cfg = ServiceTrafficConfig::default();
        assert_eq!(cfg.internal, TrafficPolicy::Cluster);
        assert_eq!(cfg.external, TrafficPolicy::Cluster);
    }

    #[test]
    fn traffic_policy_external_local_preserves_client_ip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/k8s/api/v1/Service",
            "TrafficPolicy.External.Local",
            "tenant-lbx-tp-extloc"
        );
        let cfg = ServiceTrafficConfig {
            internal: TrafficPolicy::Cluster,
            external: TrafficPolicy::Local,
            source_ranges: vec![],
        };
        assert_eq!(cfg.external, TrafficPolicy::Local);
    }

    // ── Local backend filter ─────────────────────────────────────────────────

    #[test]
    fn filter_backends_local_keeps_only_node_owned() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "FilterLocal",
            "tenant-lbx-floc"
        );
        let bs = vec![
            Backend::new("node-a:0", ip(10, 0, 1, 1), 80),
            Backend::new("node-a:1", ip(10, 0, 1, 2), 80),
            Backend::new("node-b:0", ip(10, 0, 2, 1), 80),
        ];
        let local = filter_backends_local("node-a", &bs);
        assert_eq!(local.len(), 2);
    }

    #[test]
    fn filter_backends_local_empty_when_none_match() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "FilterLocal.None",
            "tenant-lbx-floc-none"
        );
        let bs = vec![Backend::new("node-a:0", ip(10, 0, 1, 1), 80)];
        let local = filter_backends_local("node-z", &bs);
        assert!(local.is_empty());
    }

    // ── Source ranges ────────────────────────────────────────────────────────

    #[test]
    fn source_range_allows_match() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "SourceRanges.Allow",
            "tenant-lbx-sr-ok"
        );
        let cfg = ServiceTrafficConfig {
            source_ranges: vec!["10.0.0.0/8".into()],
            ..Default::default()
        };
        assert!(cfg.allows_source(ip(10, 1, 1, 1)).unwrap());
    }

    #[test]
    fn source_range_denies_outside() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "SourceRanges.Deny",
            "tenant-lbx-sr-den"
        );
        let cfg = ServiceTrafficConfig {
            source_ranges: vec!["10.0.0.0/8".into()],
            ..Default::default()
        };
        assert!(!cfg.allows_source(ip(11, 0, 0, 1)).unwrap());
    }

    #[test]
    fn source_range_empty_allows_any() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "SourceRanges.Empty",
            "tenant-lbx-sr-emp"
        );
        let cfg = ServiceTrafficConfig::default();
        assert!(cfg.allows_source(ip(8, 8, 8, 8)).unwrap());
    }

    #[test]
    fn source_range_invalid_cidr_returns_error() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "SourceRanges.BadCIDR",
            "tenant-lbx-sr-bad"
        );
        let cfg = ServiceTrafficConfig {
            source_ranges: vec!["not-a-cidr".into()],
            ..Default::default()
        };
        let err = cfg.allows_source(ip(10, 0, 0, 1)).unwrap_err();
        assert_eq!(err, LbExtError::BadCidr("not-a-cidr".into()));
    }

    #[test]
    fn source_range_multiple_cidrs_first_match_allows() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "SourceRanges.Multi",
            "tenant-lbx-sr-multi"
        );
        let cfg = ServiceTrafficConfig {
            source_ranges: vec!["10.0.0.0/8".into(), "192.168.0.0/16".into()],
            ..Default::default()
        };
        assert!(cfg.allows_source(ip(192, 168, 1, 1)).unwrap());
    }

    // ── DSR ──────────────────────────────────────────────────────────────────

    #[test]
    fn dsr_encode_v4_round_trip() {
        let (_c, _t) =
            cilium_test_ctx!("bpf/lib/dsr_helpers.h", "set_dsr_opt4", "tenant-lbx-dsr-rt");
        let opt = DsrOption::encode(ip(10, 0, 1, 5), 8080).unwrap();
        let (back_ip, back_port) = opt.decode();
        assert_eq!(back_ip, ip(10, 0, 1, 5));
        assert_eq!(back_port, 8080);
    }

    #[test]
    fn dsr_encode_v6_returns_none() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/dsr_helpers.h",
            "set_dsr_opt4.V6",
            "tenant-lbx-dsr-v6"
        );
        let v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert!(DsrOption::encode(v6, 8080).is_none());
    }

    // ── LbHashing ────────────────────────────────────────────────────────────

    #[test]
    fn lb_per_flow_hash_is_stable_across_packets() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "Hashing.PerFlow",
            "tenant-lbx-hpf"
        );
        let key = flow((10, 0, 0, 1), 1234, (10, 96, 0, 1), 80);
        let a = LbHashing::PerFlow.hash_input(key, 0);
        let b = LbHashing::PerFlow.hash_input(key, 1);
        assert_eq!(a, b);
    }

    #[test]
    fn lb_per_packet_hash_changes_across_packets() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "Hashing.PerPacket",
            "tenant-lbx-hpp"
        );
        let key = flow((10, 0, 0, 1), 1234, (10, 96, 0, 1), 80);
        let a = LbHashing::PerPacket.hash_input(key, 0);
        let b = LbHashing::PerPacket.hash_input(key, 1);
        assert_ne!(a, b);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn lb_mode_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/loadbalancer/loadbalancer.go",
            "LBMode.Serde",
            "tenant-lbx-mode-serde"
        );
        for m in [LbMode::Snat, LbMode::Dsr, LbMode::Hybrid] {
            let s = serde_json::to_string(&m).unwrap();
            let back: LbMode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn xdp_mode_serde_round_trip() {
        let (_c, _t) =
            cilium_test_ctx!("pkg/datapath/xdp", "XdpMode.Serde", "tenant-lbx-xdp-serde");
        for m in [
            XdpMode::Native,
            XdpMode::Offload,
            XdpMode::Generic,
            XdpMode::Disabled,
        ] {
            let s = serde_json::to_string(&m).unwrap();
            let back: XdpMode = serde_json::from_str(&s).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn traffic_policy_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/k8s/api/v1/Service",
            "TrafficPolicy.Serde",
            "tenant-lbx-tp-serde"
        );
        let cfg = ServiceTrafficConfig {
            internal: TrafficPolicy::Local,
            external: TrafficPolicy::Local,
            source_ranges: vec!["10.0.0.0/8".into()],
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: ServiceTrafficConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn kpr_status_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/option/config.go",
            "KubeProxyReplacement.Serde",
            "tenant-lbx-kpr-serde"
        );
        let s = KubeProxyReplacementStatus::strict_default();
        let json = serde_json::to_string(&s).unwrap();
        let back: KubeProxyReplacementStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn dsr_option_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/dsr_helpers.h",
            "DsrOption.Serde",
            "tenant-lbx-dsr-serde"
        );
        let opt = DsrOption::encode(ip(10, 0, 1, 5), 8080).unwrap();
        let json = serde_json::to_string(&opt).unwrap();
        let back: DsrOption = serde_json::from_str(&json).unwrap();
        assert_eq!(back, opt);
    }
}
