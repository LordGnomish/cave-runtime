// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gratuitous ARP/NDP announce — pushed by the L2 announcer when it
//! claims a VIP.
//!
//! Mirrors `pkg/l2announcer/arp_announce.go` and the gratuitous-NDP
//! shape from `pkg/datapath/linux/ipv6/ndp.go::sendNeighborAdvert`.
//!
//! When a node wins a lease for a VIP, it sends a burst of gratuitous
//! ARP (or unsolicited NDP NA for v6) so neighbours flush their ARP
//! tables and start sending traffic to the new MAC. The send rate is
//! capped per-VIP to avoid storm scenarios.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnounceProto {
    GratuitousArp,
    UnsolicitedNa,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnounceFrame {
    pub vip: IpAddr,
    pub source_mac: [u8; 6],
    pub interface: String,
    pub proto: AnnounceProto,
    pub timestamp_ns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VipAnnounceState {
    pub vip: IpAddr,
    pub last_announce_ns: u64,
    pub announce_count: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AnnounceError {
    #[error("VIP `{0}` rate-limited (last announce at {1})")]
    RateLimited(IpAddr, u64),
    #[error("VIP `{0}` not registered")]
    NotRegistered(IpAddr),
    #[error("interface `{0}` not configured for L2 announce")]
    InterfaceNotConfigured(String),
    #[error("tenant {tenant} cannot mutate announce state owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct ArpAnnouncer {
    pub tenant: TenantId,
    /// Minimum interval between announces for the same VIP (ns).
    pub min_interval_ns: u64,
    /// MAC of the local node (used in source-mac of each frame).
    pub local_mac: [u8; 6],
    /// Interfaces enabled for L2 announce.
    interfaces: BTreeMap<String, ()>,
    state: BTreeMap<IpAddr, VipAnnounceState>,
    sent: Vec<AnnounceFrame>,
}

impl ArpAnnouncer {
    pub fn new(tenant: TenantId, local_mac: [u8; 6], min_interval_seconds: u64) -> Self {
        Self {
            tenant,
            local_mac,
            min_interval_ns: min_interval_seconds * 1_000_000_000,
            interfaces: BTreeMap::new(),
            state: BTreeMap::new(),
            sent: Vec::new(),
        }
    }

    pub fn enable_interface(&mut self, name: impl Into<String>) {
        self.interfaces.insert(name.into(), ());
    }

    pub fn register_vip(&mut self, vip: IpAddr) {
        self.state.entry(vip).or_insert(VipAnnounceState {
            vip,
            last_announce_ns: 0,
            announce_count: 0,
        });
    }

    pub fn deregister_vip(&mut self, vip: IpAddr) -> bool {
        self.state.remove(&vip).is_some()
    }

    pub fn registered_count(&self) -> usize {
        self.state.len()
    }

    /// Send an announce frame for `vip` on `interface`. Rate-limited per VIP.
    pub fn announce(
        &mut self,
        vip: IpAddr,
        interface: &str,
        now_ns: u64,
    ) -> Result<AnnounceFrame, AnnounceError> {
        if !self.interfaces.contains_key(interface) {
            return Err(AnnounceError::InterfaceNotConfigured(interface.to_string()));
        }
        let state = self
            .state
            .get_mut(&vip)
            .ok_or(AnnounceError::NotRegistered(vip))?;
        if state.announce_count > 0
            && now_ns.saturating_sub(state.last_announce_ns) < self.min_interval_ns
        {
            return Err(AnnounceError::RateLimited(vip, state.last_announce_ns));
        }
        let frame = AnnounceFrame {
            vip,
            source_mac: self.local_mac,
            interface: interface.to_string(),
            proto: if vip.is_ipv4() {
                AnnounceProto::GratuitousArp
            } else {
                AnnounceProto::UnsolicitedNa
            },
            timestamp_ns: now_ns,
        };
        state.last_announce_ns = now_ns;
        state.announce_count += 1;
        self.sent.push(frame.clone());
        Ok(frame)
    }

    /// Send a burst of `count` announces (used after a lease takeover);
    /// rate-limit only applies to the first frame, subsequent frames in
    /// the burst share the same timestamp window.
    pub fn announce_burst(
        &mut self,
        vip: IpAddr,
        interface: &str,
        count: u32,
        now_ns: u64,
    ) -> Result<u32, AnnounceError> {
        if !self.interfaces.contains_key(interface) {
            return Err(AnnounceError::InterfaceNotConfigured(interface.to_string()));
        }
        if count == 0 {
            return Ok(0);
        }
        // First frame respects rate limit.
        self.announce(vip, interface, now_ns)?;
        let proto = if vip.is_ipv4() {
            AnnounceProto::GratuitousArp
        } else {
            AnnounceProto::UnsolicitedNa
        };
        let state = self
            .state
            .get_mut(&vip)
            .ok_or(AnnounceError::NotRegistered(vip))?;
        for _ in 1..count {
            let frame = AnnounceFrame {
                vip,
                source_mac: self.local_mac,
                interface: interface.to_string(),
                proto,
                timestamp_ns: now_ns,
            };
            state.announce_count += 1;
            self.sent.push(frame);
        }
        Ok(count)
    }

    pub fn state_for(&self, vip: IpAddr) -> Option<&VipAnnounceState> {
        self.state.get(&vip)
    }

    pub fn drain_sent(&mut self) -> Vec<AnnounceFrame> {
        std::mem::take(&mut self.sent)
    }

    pub fn sent_count(&self) -> usize {
        self.sent.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/l2announcer/arp_announce.go", "Announcer");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn announcer(tenant: TenantId) -> ArpAnnouncer {
        ArpAnnouncer::new(tenant, [0x02, 0x00, 0x00, 0x00, 0x00, 0xAA], 1)
    }

    // ── Interface gating ────────────────────────────────────────────────────

    #[test]
    fn announce_on_unconfigured_interface_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.UnconfiguredIface",
            "tenant-arp-uif"
        );
        let mut a = announcer(tenant);
        a.register_vip(ip(203, 0, 113, 5));
        let err = a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap_err();
        assert!(matches!(err, AnnounceError::InterfaceNotConfigured(_)));
    }

    #[test]
    fn announce_on_enabled_interface_succeeds() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.OnEnabled",
            "tenant-arp-en"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        let frame = a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        assert_eq!(frame.vip, ip(203, 0, 113, 5));
    }

    // ── Registration ───────────────────────────────────────────────────────

    #[test]
    fn announce_unregistered_vip_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.NotRegistered",
            "tenant-arp-nr"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        let err = a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap_err();
        assert!(matches!(err, AnnounceError::NotRegistered(_)));
    }

    #[test]
    fn deregister_drops_vip() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Deregister",
            "tenant-arp-dr"
        );
        let mut a = announcer(tenant);
        a.register_vip(ip(203, 0, 113, 5));
        assert!(a.deregister_vip(ip(203, 0, 113, 5)));
        assert!(!a.deregister_vip(ip(203, 0, 113, 5)));
    }

    #[test]
    fn registered_count_tracks_register() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/l2announcer/arp_announce.go", "Count", "tenant-arp-c");
        let mut a = announcer(tenant);
        for i in 1..=3u8 {
            a.register_vip(ip(203, 0, 113, i));
        }
        assert_eq!(a.registered_count(), 3);
    }

    // ── Rate limit ─────────────────────────────────────────────────────────

    #[test]
    fn second_announce_within_interval_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.RateLimit",
            "tenant-arp-rl"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        let err = a
            .announce(ip(203, 0, 113, 5), "eth0", 500_000_000)
            .unwrap_err();
        assert!(matches!(err, AnnounceError::RateLimited(_, _)));
    }

    #[test]
    fn announce_after_interval_succeeds() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.AfterInterval",
            "tenant-arp-ai"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        a.announce(ip(203, 0, 113, 5), "eth0", 1_500_000_000)
            .unwrap();
        assert_eq!(a.state_for(ip(203, 0, 113, 5)).unwrap().announce_count, 2);
    }

    // ── IPv4 vs IPv6 protocol ──────────────────────────────────────────────

    #[test]
    fn announce_v4_uses_gratuitous_arp() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.V4Proto",
            "tenant-arp-v4"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        let frame = a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        assert_eq!(frame.proto, AnnounceProto::GratuitousArp);
    }

    #[test]
    fn announce_v6_uses_unsolicited_na() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.V6Proto",
            "tenant-arp-v6"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        let v6: IpAddr = "2001:db8::5".parse().unwrap();
        a.register_vip(v6);
        let frame = a.announce(v6, "eth0", 0).unwrap();
        assert_eq!(frame.proto, AnnounceProto::UnsolicitedNa);
    }

    // ── Source MAC ─────────────────────────────────────────────────────────

    #[test]
    fn announce_uses_local_node_mac() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.SourceMac",
            "tenant-arp-sm"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        let frame = a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        assert_eq!(frame.source_mac, [0x02, 0x00, 0x00, 0x00, 0x00, 0xAA]);
    }

    // ── Burst ──────────────────────────────────────────────────────────────

    #[test]
    fn announce_burst_emits_n_frames() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Announce.Burst",
            "tenant-arp-b"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        let n = a.announce_burst(ip(203, 0, 113, 5), "eth0", 3, 0).unwrap();
        assert_eq!(n, 3);
        assert_eq!(a.sent_count(), 3);
    }

    #[test]
    fn announce_burst_respects_first_frame_rate_limit() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Burst.RateLimit",
            "tenant-arp-brl"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        let err = a
            .announce_burst(ip(203, 0, 113, 5), "eth0", 5, 100_000_000)
            .unwrap_err();
        assert!(matches!(err, AnnounceError::RateLimited(_, _)));
    }

    #[test]
    fn announce_burst_zero_returns_zero() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Burst.Zero",
            "tenant-arp-bz"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        let n = a.announce_burst(ip(203, 0, 113, 5), "eth0", 0, 0).unwrap();
        assert_eq!(n, 0);
    }

    // ── Sent buffer ────────────────────────────────────────────────────────

    #[test]
    fn drain_sent_returns_recorded_frames() {
        let (_c, tenant) =
            cilium_test_ctx!("pkg/l2announcer/arp_announce.go", "Drain", "tenant-arp-d");
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        let frames = a.drain_sent();
        assert_eq!(frames.len(), 1);
        assert_eq!(a.sent_count(), 0);
    }

    // ── State counters ─────────────────────────────────────────────────────

    #[test]
    fn announce_increments_per_vip_counter() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "State.Counter",
            "tenant-arp-cnt"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.register_vip(ip(203, 0, 113, 5));
        a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        a.announce(ip(203, 0, 113, 5), "eth0", 2_000_000_000)
            .unwrap();
        assert_eq!(a.state_for(ip(203, 0, 113, 5)).unwrap().announce_count, 2);
    }

    #[test]
    fn state_for_unknown_vip_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "State.NotFound",
            "tenant-arp-snf"
        );
        let a = announcer(tenant);
        assert!(a.state_for(ip(1, 2, 3, 4)).is_none());
    }

    // ── Interface enable / multi ───────────────────────────────────────────

    #[test]
    fn multiple_interfaces_announce_per_iface() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "MultiIface",
            "tenant-arp-mi"
        );
        let mut a = announcer(tenant);
        a.enable_interface("eth0");
        a.enable_interface("eth1");
        a.register_vip(ip(203, 0, 113, 5));
        a.announce(ip(203, 0, 113, 5), "eth0", 0).unwrap();
        // Rate-limited per VIP regardless of interface.
        let err = a
            .announce(ip(203, 0, 113, 5), "eth1", 100_000_000)
            .unwrap_err();
        assert!(matches!(err, AnnounceError::RateLimited(_, _)));
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn announce_frame_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Frame.Serde",
            "tenant-arp-fserde"
        );
        let f = AnnounceFrame {
            vip: ip(203, 0, 113, 5),
            source_mac: [1, 2, 3, 4, 5, 6],
            interface: "eth0".into(),
            proto: AnnounceProto::GratuitousArp,
            timestamp_ns: 100,
        };
        let s = serde_json::to_string(&f).unwrap();
        let back: AnnounceFrame = serde_json::from_str(&s).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn vip_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "State.Serde",
            "tenant-arp-sserde"
        );
        let s = VipAnnounceState {
            vip: ip(203, 0, 113, 5),
            last_announce_ns: 100,
            announce_count: 5,
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: VipAnnounceState = serde_json::from_str(&j).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn announce_proto_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/l2announcer/arp_announce.go",
            "Proto.Serde",
            "tenant-arp-pserde"
        );
        for p in [AnnounceProto::GratuitousArp, AnnounceProto::UnsolicitedNa] {
            let s = serde_json::to_string(&p).unwrap();
            let back: AnnounceProto = serde_json::from_str(&s).unwrap();
            assert_eq!(back, p);
        }
    }
}
