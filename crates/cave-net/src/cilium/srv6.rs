// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SRv6 (Segment Routing v6) — IPv6-based service chaining.
//!
//! Mirrors `pkg/srv6/manager.go` and the BPF helpers in
//! `bpf/lib/srv6.h`. SRv6 encodes a list of "Segment Identifiers"
//! (SIDs, themselves IPv6 addresses) in an SRH header so each hop pops
//! a SID and forwards according to the action encoded in the SID's
//! lower bits ("function" portion).
//!
//! Cilium uses SRv6 to:
//!
//! * Implement L3VPNs over the cluster (tenant per VRF, encap inner
//!   packet inside the outer SRv6 packet).
//! * Chain endpoints — service A → SID(B) → SID(C) → final dst, each
//!   intermediate node performing the configured behaviour.
//!
//! Behaviour codes (RFC 8986 §4):
//!
//! * `End` — pseudonym for "next segment, then forward".
//! * `End.DX4` / `End.DX6` — decap and L3 cross-connect to a configured
//!   nexthop (IPv4/IPv6).
//! * `End.DT4` / `End.DT6` — decap and table lookup in a specific VRF.
//! * `End.B6.Encaps` — bind to a new SRv6 policy and encap.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Srv6Behavior {
    End,
    EndDx4 { nexthop: std::net::Ipv4Addr },
    EndDx6 { nexthop: Ipv6Addr },
    EndDt4 { vrf_id: u32 },
    EndDt6 { vrf_id: u32 },
    EndB6Encaps { sid_list: u8 /* number of SIDs to follow, kept abstract */ },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sid(pub Ipv6Addr);

impl Sid {
    /// Locator portion = upper 96 bits (per RFC 8986 default schema).
    pub fn locator(self) -> [u8; 12] {
        let octets = self.0.octets();
        let mut out = [0u8; 12];
        out.copy_from_slice(&octets[..12]);
        out
    }
    /// Function portion = lower 32 bits.
    pub fn function(self) -> u32 {
        let octets = self.0.octets();
        u32::from_be_bytes([octets[12], octets[13], octets[14], octets[15]])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidList {
    pub sids: Vec<Sid>,
    /// 0-based index of the *current* SID. Each hop decrements
    /// `segments_left` so `current` points to the next entry.
    pub segments_left: u8,
}

impl SidList {
    pub fn new(sids: Vec<Sid>) -> Self {
        let n = sids.len() as u8;
        Self { sids, segments_left: n.saturating_sub(1) }
    }
    pub fn current(&self) -> Option<Sid> {
        if self.sids.is_empty() {
            return None;
        }
        let idx = (self.sids.len() as i32 - 1 - self.segments_left as i32).max(0) as usize;
        self.sids.get(idx).copied()
    }
    pub fn at_end(&self) -> bool {
        self.segments_left == 0
    }
    pub fn pop(&mut self) {
        if self.segments_left > 0 {
            self.segments_left -= 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locator {
    pub prefix: Ipv6Addr,
    pub prefix_len: u8,
    pub behavior: Srv6Behavior,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VrfBinding {
    pub vrf_id: u32,
    pub pod_cidr_v4: Option<String>,
    pub pod_cidr_v6: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressPolicy {
    pub name: String,
    pub destination_cidr: String,
    pub sid_list: Vec<Sid>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Srv6Error {
    #[error("locator `{0}` not found")]
    LocatorNotFound(String),
    #[error("VRF `{0}` not registered")]
    VrfNotFound(u32),
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("policy `{0}` not found")]
    PolicyNotFound(String),
    #[error("tenant {tenant} cannot mutate SRv6 manager owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct Srv6Manager {
    pub tenant: TenantId,
    locators: HashMap<Ipv6Addr, Locator>,
    vrfs: HashMap<u32, VrfBinding>,
    egress_policies: HashMap<String, EgressPolicy>,
}

impl Srv6Manager {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            locators: HashMap::new(),
            vrfs: HashMap::new(),
            egress_policies: HashMap::new(),
        }
    }

    pub fn upsert_locator(&mut self, locator: Locator) {
        self.locators.insert(locator.prefix, locator);
    }

    pub fn lookup_locator(&self, sid: Sid) -> Option<&Locator> {
        // Walk all locators and return the longest-prefix match.
        let target = sid.0;
        let mut best: Option<&Locator> = None;
        let mut best_len = 0u8;
        for l in self.locators.values() {
            if locator_matches(target, l.prefix, l.prefix_len) && l.prefix_len >= best_len {
                best = Some(l);
                best_len = l.prefix_len;
            }
        }
        best
    }

    pub fn locator_count(&self) -> usize {
        self.locators.len()
    }

    pub fn upsert_vrf(&mut self, vrf: VrfBinding) -> Result<(), Srv6Error> {
        if let Some(c) = &vrf.pod_cidr_v4 {
            ipnet::IpNet::from_str(c).map_err(|_| Srv6Error::BadCidr(c.clone()))?;
        }
        if let Some(c) = &vrf.pod_cidr_v6 {
            ipnet::IpNet::from_str(c).map_err(|_| Srv6Error::BadCidr(c.clone()))?;
        }
        self.vrfs.insert(vrf.vrf_id, vrf);
        Ok(())
    }

    pub fn lookup_vrf(&self, vrf_id: u32) -> Option<&VrfBinding> {
        self.vrfs.get(&vrf_id)
    }

    pub fn vrf_count(&self) -> usize {
        self.vrfs.len()
    }

    pub fn upsert_policy(&mut self, p: EgressPolicy) -> Result<(), Srv6Error> {
        ipnet::IpNet::from_str(&p.destination_cidr)
            .map_err(|_| Srv6Error::BadCidr(p.destination_cidr.clone()))?;
        self.egress_policies.insert(p.name.clone(), p);
        Ok(())
    }

    pub fn remove_policy(&mut self, name: &str) -> Result<(), Srv6Error> {
        self.egress_policies.remove(name).ok_or_else(|| Srv6Error::PolicyNotFound(name.to_string()))?;
        Ok(())
    }

    pub fn policy_count(&self) -> usize {
        self.egress_policies.len()
    }

    /// Resolve the SID list to push for a packet heading to `dst`. Returns
    /// `None` if no policy matches.
    pub fn lookup_egress_policy(&self, dst: IpAddr) -> Option<&EgressPolicy> {
        for p in self.egress_policies.values() {
            let net = match ipnet::IpNet::from_str(&p.destination_cidr) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if net.contains(&dst) {
                return Some(p);
            }
        }
        None
    }
}

fn locator_matches(addr: Ipv6Addr, prefix: Ipv6Addr, prefix_len: u8) -> bool {
    let a = u128::from_be_bytes(addr.octets());
    let p = u128::from_be_bytes(prefix.octets());
    if prefix_len == 0 {
        return true;
    }
    let mask = !0u128 << (128 - prefix_len);
    (a & mask) == (p & mask)
}

use std::str::FromStr;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/srv6/manager.go", "Manager");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn v6(s: &str) -> Ipv6Addr {
        s.parse().unwrap()
    }

    fn mgr(tenant: TenantId) -> Srv6Manager {
        Srv6Manager::new(tenant)
    }

    // ── SID ─────────────────────────────────────────────────────────────────

    #[test]
    fn sid_locator_returns_upper_96_bits() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "Sid.Locator", "tenant-srv6-loc");
        let sid = Sid(v6("fd00:db8:abcd:1234::1"));
        let loc = sid.locator();
        // Locator should match the upper 12 bytes.
        assert_eq!(&loc[..2], &[0xfd, 0x00]);
        assert_eq!(&loc[10..12], &[0x00, 0x00]);
    }

    #[test]
    fn sid_function_returns_lower_32_bits() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "Sid.Function", "tenant-srv6-fn");
        let sid = Sid(v6("fd00::abcd:1234"));
        assert_eq!(sid.function(), 0xabcd_1234);
    }

    // ── SidList traversal ──────────────────────────────────────────────────

    #[test]
    fn sidlist_new_initialises_segments_left_to_n_minus_1() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "SidList.New", "tenant-srv6-sln");
        let sl = SidList::new(vec![Sid(v6("fd00::1")), Sid(v6("fd00::2")), Sid(v6("fd00::3"))]);
        assert_eq!(sl.segments_left, 2);
    }

    #[test]
    fn sidlist_current_starts_at_first_sid() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "SidList.Current", "tenant-srv6-sc");
        let sl = SidList::new(vec![Sid(v6("fd00::1")), Sid(v6("fd00::2"))]);
        assert_eq!(sl.current(), Some(Sid(v6("fd00::1"))));
    }

    #[test]
    fn sidlist_pop_advances_segments() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "SidList.Pop", "tenant-srv6-pop");
        let mut sl = SidList::new(vec![Sid(v6("fd00::1")), Sid(v6("fd00::2"))]);
        sl.pop();
        assert_eq!(sl.segments_left, 0);
        assert_eq!(sl.current(), Some(Sid(v6("fd00::2"))));
    }

    #[test]
    fn sidlist_at_end_when_segments_left_zero() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "SidList.AtEnd", "tenant-srv6-end");
        let sl = SidList::new(vec![Sid(v6("fd00::1"))]);
        assert!(sl.at_end());
    }

    #[test]
    fn sidlist_pop_at_end_is_idempotent() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "SidList.PopAtEnd", "tenant-srv6-popend");
        let mut sl = SidList::new(vec![Sid(v6("fd00::1"))]);
        sl.pop();
        sl.pop();
        assert_eq!(sl.segments_left, 0);
    }

    #[test]
    fn sidlist_empty_current_returns_none() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/srv6.h", "SidList.Empty", "tenant-srv6-empty");
        let sl = SidList::new(vec![]);
        assert!(sl.current().is_none());
    }

    // ── Locator ────────────────────────────────────────────────────────────

    #[test]
    fn locator_match_uses_longest_prefix() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LookupLocator", "tenant-srv6-lp");
        let mut m = mgr(tenant);
        m.upsert_locator(Locator {
            prefix: v6("fd00::"), prefix_len: 16,
            behavior: Srv6Behavior::End,
        });
        m.upsert_locator(Locator {
            prefix: v6("fd00:db8::"), prefix_len: 32,
            behavior: Srv6Behavior::EndDt4 { vrf_id: 7 },
        });
        let l = m.lookup_locator(Sid(v6("fd00:db8::1234"))).unwrap();
        assert!(matches!(l.behavior, Srv6Behavior::EndDt4 { vrf_id: 7 }));
    }

    #[test]
    fn locator_match_falls_through_to_shorter_prefix() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LookupLocator.Fallthrough", "tenant-srv6-lpf");
        let mut m = mgr(tenant);
        m.upsert_locator(Locator {
            prefix: v6("fd00::"), prefix_len: 16,
            behavior: Srv6Behavior::End,
        });
        let l = m.lookup_locator(Sid(v6("fd00:99::1234"))).unwrap();
        assert!(matches!(l.behavior, Srv6Behavior::End));
    }

    #[test]
    fn locator_no_match_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LookupLocator.NotFound", "tenant-srv6-lpnf");
        let m = mgr(tenant);
        assert!(m.lookup_locator(Sid(v6("fd00::1"))).is_none());
    }

    #[test]
    fn locator_count_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LocatorCount", "tenant-srv6-lcnt");
        let mut m = mgr(tenant);
        for i in 0..3u8 {
            m.upsert_locator(Locator {
                prefix: v6(&format!("fd00:{i}::")),
                prefix_len: 32,
                behavior: Srv6Behavior::End,
            });
        }
        assert_eq!(m.locator_count(), 3);
    }

    // ── VRF binding ────────────────────────────────────────────────────────

    #[test]
    fn vrf_upsert_with_valid_cidrs_succeeds() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "UpsertVRF", "tenant-srv6-vrf");
        let mut m = mgr(tenant);
        m.upsert_vrf(VrfBinding {
            vrf_id: 7,
            pod_cidr_v4: Some("10.244.7.0/24".into()),
            pod_cidr_v6: Some("fd00:7::/64".into()),
        }).unwrap();
        assert_eq!(m.lookup_vrf(7).unwrap().vrf_id, 7);
    }

    #[test]
    fn vrf_upsert_with_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "UpsertVRF.BadCidr", "tenant-srv6-vrfbad");
        let mut m = mgr(tenant);
        let err = m.upsert_vrf(VrfBinding {
            vrf_id: 7,
            pod_cidr_v4: Some("not-a-cidr".into()),
            pod_cidr_v6: None,
        }).unwrap_err();
        assert_eq!(err, Srv6Error::BadCidr("not-a-cidr".into()));
    }

    #[test]
    fn vrf_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LookupVRF.NotFound", "tenant-srv6-vrfnf");
        let m = mgr(tenant);
        assert!(m.lookup_vrf(99).is_none());
    }

    // ── Egress policies ─────────────────────────────────────────────────────

    #[test]
    fn egress_policy_match_pushes_sid_list() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LookupEgressPolicy", "tenant-srv6-eg");
        let mut m = mgr(tenant);
        m.upsert_policy(EgressPolicy {
            name: "to-vpn".into(),
            destination_cidr: "10.10.0.0/16".into(),
            sid_list: vec![Sid(v6("fd00:db8::1")), Sid(v6("fd00:db8::2"))],
        }).unwrap();
        let p = m.lookup_egress_policy(IpAddr::V4(std::net::Ipv4Addr::new(10, 10, 5, 1))).unwrap();
        assert_eq!(p.sid_list.len(), 2);
    }

    #[test]
    fn egress_policy_no_match_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "LookupEgressPolicy.NoMatch", "tenant-srv6-egnf");
        let mut m = mgr(tenant);
        m.upsert_policy(EgressPolicy {
            name: "p".into(),
            destination_cidr: "10.0.0.0/8".into(),
            sid_list: vec![Sid(v6("fd00::1"))],
        }).unwrap();
        let p = m.lookup_egress_policy(IpAddr::V4(std::net::Ipv4Addr::new(8, 8, 8, 8)));
        assert!(p.is_none());
    }

    #[test]
    fn egress_policy_with_bad_cidr_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "UpsertPolicy.BadCidr", "tenant-srv6-egbc");
        let mut m = mgr(tenant);
        let err = m.upsert_policy(EgressPolicy {
            name: "p".into(),
            destination_cidr: "nope".into(),
            sid_list: vec![],
        }).unwrap_err();
        assert!(matches!(err, Srv6Error::BadCidr(_)));
    }

    #[test]
    fn egress_policy_remove_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "RemovePolicy", "tenant-srv6-rmp");
        let mut m = mgr(tenant);
        m.upsert_policy(EgressPolicy {
            name: "p".into(),
            destination_cidr: "10.0.0.0/8".into(),
            sid_list: vec![],
        }).unwrap();
        m.remove_policy("p").unwrap();
        assert_eq!(m.policy_count(), 0);
    }

    #[test]
    fn egress_policy_remove_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!("pkg/srv6/manager.go", "RemovePolicy.NotFound", "tenant-srv6-rmpnf");
        let mut m = mgr(tenant);
        let err = m.remove_policy("ghost").unwrap_err();
        assert!(matches!(err, Srv6Error::PolicyNotFound(_)));
    }

    // ── Behaviour variants ──────────────────────────────────────────────────

    #[test]
    fn behavior_end_dx4_carries_nexthop() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "Behavior.EndDX4", "tenant-srv6-dx4");
        let b = Srv6Behavior::EndDx4 { nexthop: std::net::Ipv4Addr::new(10, 0, 0, 1) };
        match b {
            Srv6Behavior::EndDx4 { nexthop } => assert_eq!(nexthop, std::net::Ipv4Addr::new(10, 0, 0, 1)),
            _ => panic!(),
        }
    }

    #[test]
    fn behavior_end_dt6_carries_vrf() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "Behavior.EndDT6", "tenant-srv6-dt6");
        let b = Srv6Behavior::EndDt6 { vrf_id: 42 };
        match b {
            Srv6Behavior::EndDt6 { vrf_id } => assert_eq!(vrf_id, 42),
            _ => panic!(),
        }
    }

    #[test]
    fn behavior_end_b6_encaps_records_sid_count() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "Behavior.EndB6Encaps", "tenant-srv6-b6");
        let b = Srv6Behavior::EndB6Encaps { sid_list: 3 };
        match b {
            Srv6Behavior::EndB6Encaps { sid_list } => assert_eq!(sid_list, 3),
            _ => panic!(),
        }
    }

    #[test]
    fn behavior_end_dx6_carries_v6_nexthop() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "Behavior.EndDX6", "tenant-srv6-dx6");
        let b = Srv6Behavior::EndDx6 { nexthop: v6("fd00::99") };
        match b {
            Srv6Behavior::EndDx6 { nexthop } => assert_eq!(nexthop, v6("fd00::99")),
            _ => panic!(),
        }
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn srv6_behavior_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "Behavior.Serde", "tenant-srv6-bserde");
        for b in [
            Srv6Behavior::End,
            Srv6Behavior::EndDx4 { nexthop: std::net::Ipv4Addr::new(10, 0, 0, 1) },
            Srv6Behavior::EndDt4 { vrf_id: 7 },
            Srv6Behavior::EndDt6 { vrf_id: 7 },
            Srv6Behavior::EndB6Encaps { sid_list: 3 },
        ] {
            let s = serde_json::to_string(&b).unwrap();
            let back: Srv6Behavior = serde_json::from_str(&s).unwrap();
            assert_eq!(back, b);
        }
    }

    #[test]
    fn sidlist_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "SidList.Serde", "tenant-srv6-slserde");
        let sl = SidList::new(vec![Sid(v6("fd00::1")), Sid(v6("fd00::2"))]);
        let s = serde_json::to_string(&sl).unwrap();
        let back: SidList = serde_json::from_str(&s).unwrap();
        assert_eq!(back, sl);
    }

    #[test]
    fn egress_policy_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "EgressPolicy.Serde", "tenant-srv6-eserde");
        let p = EgressPolicy {
            name: "to-vpn".into(),
            destination_cidr: "10.10.0.0/16".into(),
            sid_list: vec![Sid(v6("fd00:db8::1"))],
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: EgressPolicy = serde_json::from_str(&s).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn vrf_binding_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/srv6/manager.go", "VrfBinding.Serde", "tenant-srv6-vserde");
        let v = VrfBinding {
            vrf_id: 7,
            pod_cidr_v4: Some("10.244.7.0/24".into()),
            pod_cidr_v6: Some("fd00:7::/64".into()),
        };
        let s = serde_json::to_string(&v).unwrap();
        let back: VrfBinding = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
    }
}
