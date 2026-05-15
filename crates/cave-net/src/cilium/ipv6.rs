// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! IPv6 datapath + Neighbor Discovery (ND) proxy.
//!
//! Mirrors `bpf/bpf_lxc.c::handle_ipv6` plus
//! `pkg/datapath/linux/ipv6/ndp.go` (the agent-side ND proxy state).
//!
//! Covers:
//!
//! * NDP message types (Router Solicit/Advert, Neighbor Solicit/Advert,
//!   Redirect) keyed numerically per RFC 4861.
//! * Neighbor cache states (`NUD_INCOMPLETE`, `NUD_REACHABLE`, `NUD_STALE`,
//!   `NUD_DELAY`, `NUD_PROBE`, `NUD_FAILED`) and their transitions.
//! * Cilium ND proxy: when running in tunnel mode without on-link L2,
//!   cilium-agent answers Neighbor Solicits for remote pod IPs with the
//!   node's MAC so the kernel forwards the encap to the right interface.
//! * Per-pod IPv6 address allocation hints (SLAAC vs DHCPv6 — Cilium
//!   defaults to SLAAC-like deterministic addressing within the pod CIDR).

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};

// ── NDP message types (RFC 4861) ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NdpType {
    /// ICMPv6 type 133 — Router Solicitation.
    RouterSolicitation,
    /// ICMPv6 type 134 — Router Advertisement.
    RouterAdvertisement,
    /// ICMPv6 type 135 — Neighbor Solicitation.
    NeighborSolicitation,
    /// ICMPv6 type 136 — Neighbor Advertisement.
    NeighborAdvertisement,
    /// ICMPv6 type 137 — Redirect.
    Redirect,
}

impl NdpType {
    pub fn icmp_type(self) -> u8 {
        match self {
            NdpType::RouterSolicitation => 133,
            NdpType::RouterAdvertisement => 134,
            NdpType::NeighborSolicitation => 135,
            NdpType::NeighborAdvertisement => 136,
            NdpType::Redirect => 137,
        }
    }
    pub fn from_icmp_type(t: u8) -> Option<Self> {
        Some(match t {
            133 => NdpType::RouterSolicitation,
            134 => NdpType::RouterAdvertisement,
            135 => NdpType::NeighborSolicitation,
            136 => NdpType::NeighborAdvertisement,
            137 => NdpType::Redirect,
            _ => return None,
        })
    }
}

// ── Neighbor cache states (RFC 4861 §7.3.2) ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NudState {
    /// `NUD_INCOMPLETE` — solicitation in flight.
    Incomplete,
    /// `NUD_REACHABLE` — confirmed within the reachable window.
    Reachable,
    /// `NUD_STALE` — was reachable but the timer expired; first packet
    /// triggers `Delay` → `Probe`.
    Stale,
    /// `NUD_DELAY` — a packet was sent recently; awaiting upper-layer
    /// confirmation.
    Delay,
    /// `NUD_PROBE` — actively sending unicast NS.
    Probe,
    /// `NUD_FAILED` — gave up.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    pub fn zero() -> Self {
        Self([0; 6])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighborEntry {
    pub ip: Ipv6Addr,
    pub mac: MacAddr,
    pub state: NudState,
    pub last_update_ns: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Ipv6Error {
    #[error("neighbor entry for {0} not found")]
    NeighborNotFound(Ipv6Addr),
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("address {0} is not in pod CIDR `{1}`")]
    AddressOutOfCidr(Ipv6Addr, String),
    #[error("tenant {tenant} cannot mutate IPv6 manager owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct NeighborCache {
    pub tenant: TenantId,
    entries: HashMap<Ipv6Addr, NeighborEntry>,
    /// Reachable timer (default 30000ms = 30s per RFC 4861).
    pub reachable_ns: u64,
    /// Delay-first-probe (5s default).
    pub delay_ns: u64,
}

impl NeighborCache {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant, entries: HashMap::new(),
            reachable_ns: 30 * 1_000_000_000,
            delay_ns: 5 * 1_000_000_000,
        }
    }

    /// Solicitation hit — moves entry into Incomplete (fresh) or
    /// Probe (existing entry due to Stale/Delay).
    pub fn solicit(&mut self, ip: Ipv6Addr, now_ns: u64) -> NudState {
        let next = match self.entries.get(&ip).map(|e| e.state) {
            None => NudState::Incomplete,
            Some(NudState::Stale) | Some(NudState::Delay) => NudState::Probe,
            Some(other) => other,
        };
        let entry = self.entries.entry(ip).or_insert(NeighborEntry {
            ip, mac: MacAddr::zero(), state: next, last_update_ns: now_ns,
        });
        entry.state = next;
        entry.last_update_ns = now_ns;
        next
    }

    /// Got an advertisement (NA) from `ip` with `mac`. Move to Reachable.
    pub fn confirm(&mut self, ip: Ipv6Addr, mac: MacAddr, now_ns: u64) {
        self.entries.insert(ip, NeighborEntry {
            ip, mac, state: NudState::Reachable, last_update_ns: now_ns,
        });
    }

    /// Tick the cache forward — Reachable entries past the reachable
    /// window become Stale. Returns the count transitioned.
    pub fn tick(&mut self, now_ns: u64) -> usize {
        let mut n = 0;
        for e in self.entries.values_mut() {
            let elapsed = now_ns.saturating_sub(e.last_update_ns);
            if matches!(e.state, NudState::Reachable) && elapsed >= self.reachable_ns {
                e.state = NudState::Stale;
                n += 1;
            } else if matches!(e.state, NudState::Probe) && elapsed >= self.delay_ns {
                e.state = NudState::Failed;
                n += 1;
            }
        }
        n
    }

    pub fn lookup(&self, ip: Ipv6Addr) -> Option<&NeighborEntry> {
        self.entries.get(&ip)
    }

    pub fn remove(&mut self, ip: Ipv6Addr) -> bool {
        self.entries.remove(&ip).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

// ── ND proxy ─────────────────────────────────────────────────────────────────
//
// When running in tunnel mode with no L2 overlay (e.g. Cilium without
// L2-announce), the kernel doesn't have a route to remote pod IPs. The
// agent intercepts NS messages targeting remote pod IPs and answers
// with the node's own MAC so the kernel forwards the inner packet to
// the cilium_host interface, which then encaps.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NdProxyEntry {
    pub remote_pod_ip: Ipv6Addr,
    pub remote_node_ip: IpAddr,
    pub local_node_mac: MacAddr,
}

#[derive(Debug)]
pub struct NdProxy {
    pub tenant: TenantId,
    proxies: HashMap<Ipv6Addr, NdProxyEntry>,
}

impl NdProxy {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, proxies: HashMap::new() }
    }

    pub fn upsert(&mut self, entry: NdProxyEntry) {
        self.proxies.insert(entry.remote_pod_ip, entry);
    }

    pub fn remove(&mut self, ip: Ipv6Addr) -> bool {
        self.proxies.remove(&ip).is_some()
    }

    pub fn entry_count(&self) -> usize {
        self.proxies.len()
    }

    /// Answer an NS for `target_ip` if we have a proxy entry. Returns
    /// the MAC to put in the NA, or `None` to drop / pass through.
    pub fn answer_neighbor_solicit(&self, target_ip: Ipv6Addr) -> Option<MacAddr> {
        self.proxies.get(&target_ip).map(|e| e.local_node_mac)
    }
}

// ── Pod IPv6 allocation (SLAAC-style) ───────────────────────────────────────

/// Generate a deterministic /128 from a `/64` pod CIDR + a 64-bit suffix.
/// Mirrors the SLAAC EUI-64 logic Cilium uses for pod IPv6 addresses
/// (`pkg/ipam/ipv6/slaac.go`).
pub fn slaac_address(cidr: &str, suffix: u64) -> Result<Ipv6Addr, Ipv6Error> {
    let net = ipnet::IpNet::from_str(cidr).map_err(|_| Ipv6Error::BadCidr(cidr.to_string()))?;
    let ipnet::IpNet::V6(v6) = net else {
        return Err(Ipv6Error::BadCidr(cidr.to_string()));
    };
    if v6.prefix_len() > 64 {
        return Err(Ipv6Error::BadCidr(cidr.to_string()));
    }
    let prefix_bytes = v6.network().octets();
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&prefix_bytes[..8]);
    out[8..].copy_from_slice(&suffix.to_be_bytes());
    Ok(Ipv6Addr::from(out))
}

use std::str::FromStr;

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/datapath/linux/ipv6/ndp.go", "NDProxy");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn v6(s: &str) -> Ipv6Addr {
        s.parse().unwrap()
    }

    fn mac(b: u8) -> MacAddr {
        MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, b])
    }

    // ── NdpType numeric mapping ──────────────────────────────────────────────

    #[test]
    fn ndp_icmp_types_match_rfc_4861() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/icmp6.h", "ICMPv6.NDPType", "tenant-v6-rfc");
        assert_eq!(NdpType::RouterSolicitation.icmp_type(), 133);
        assert_eq!(NdpType::RouterAdvertisement.icmp_type(), 134);
        assert_eq!(NdpType::NeighborSolicitation.icmp_type(), 135);
        assert_eq!(NdpType::NeighborAdvertisement.icmp_type(), 136);
        assert_eq!(NdpType::Redirect.icmp_type(), 137);
    }

    #[test]
    fn ndp_from_icmp_type_round_trip() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/icmp6.h", "ICMPv6.Parse", "tenant-v6-parse");
        for t in [
            NdpType::RouterSolicitation,
            NdpType::RouterAdvertisement,
            NdpType::NeighborSolicitation,
            NdpType::NeighborAdvertisement,
            NdpType::Redirect,
        ] {
            assert_eq!(NdpType::from_icmp_type(t.icmp_type()), Some(t));
        }
    }

    #[test]
    fn ndp_unknown_icmp_type_returns_none() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/icmp6.h", "ICMPv6.Unknown", "tenant-v6-unk");
        assert!(NdpType::from_icmp_type(99).is_none());
        assert!(NdpType::from_icmp_type(0).is_none());
        assert!(NdpType::from_icmp_type(255).is_none());
    }

    // ── Neighbor cache state machine ─────────────────────────────────────────

    #[test]
    fn neigh_solicit_fresh_entry_starts_incomplete() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Solicit.Fresh", "tenant-v6-frnew");
        let mut c = NeighborCache::new(tenant);
        let s = c.solicit(v6("fd00::1"), 100);
        assert_eq!(s, NudState::Incomplete);
    }

    #[test]
    fn neigh_confirm_moves_to_reachable() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Confirm.Reachable", "tenant-v6-conf");
        let mut c = NeighborCache::new(tenant);
        c.confirm(v6("fd00::1"), mac(1), 100);
        assert_eq!(c.lookup(v6("fd00::1")).unwrap().state, NudState::Reachable);
    }

    #[test]
    fn neigh_tick_moves_reachable_to_stale_after_window() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Tick.Stale", "tenant-v6-stale");
        let mut c = NeighborCache::new(tenant);
        c.confirm(v6("fd00::1"), mac(1), 0);
        let now = c.reachable_ns + 1;
        let n = c.tick(now);
        assert_eq!(n, 1);
        assert_eq!(c.lookup(v6("fd00::1")).unwrap().state, NudState::Stale);
    }

    #[test]
    fn neigh_tick_keeps_recent_reachable() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Tick.NoStale", "tenant-v6-fresh");
        let mut c = NeighborCache::new(tenant);
        c.confirm(v6("fd00::1"), mac(1), 0);
        let n = c.tick(c.reachable_ns / 2);
        assert_eq!(n, 0);
        assert_eq!(c.lookup(v6("fd00::1")).unwrap().state, NudState::Reachable);
    }

    #[test]
    fn neigh_solicit_on_stale_moves_to_probe() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Solicit.StaleToProbe", "tenant-v6-stp");
        let mut c = NeighborCache::new(tenant);
        c.confirm(v6("fd00::1"), mac(1), 0);
        c.tick(c.reachable_ns + 1);
        let s = c.solicit(v6("fd00::1"), c.reachable_ns + 2);
        assert_eq!(s, NudState::Probe);
    }

    #[test]
    fn neigh_probe_timeout_moves_to_failed() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Probe.Failed", "tenant-v6-pfail");
        let mut c = NeighborCache::new(tenant);
        let now = 1_000_000_000;
        c.solicit(v6("fd00::1"), now);
        // Force into probe by resoliciting after stale (manual setup).
        if let Some(e) = c.entries.get_mut(&v6("fd00::1")) {
            e.state = NudState::Probe;
            e.last_update_ns = now;
        }
        c.tick(now + c.delay_ns + 1);
        assert_eq!(c.lookup(v6("fd00::1")).unwrap().state, NudState::Failed);
    }

    #[test]
    fn neigh_remove_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Remove", "tenant-v6-rm");
        let mut c = NeighborCache::new(tenant);
        c.confirm(v6("fd00::1"), mac(1), 0);
        assert!(c.remove(v6("fd00::1")));
        assert!(c.lookup(v6("fd00::1")).is_none());
    }

    #[test]
    fn neigh_remove_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Remove.NotFound", "tenant-v6-rmnf");
        let mut c = NeighborCache::new(tenant);
        assert!(!c.remove(v6("fd00::1")));
    }

    // ── ND proxy ─────────────────────────────────────────────────────────────

    #[test]
    fn ndproxy_answers_with_local_node_mac() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Proxy.Answer", "tenant-v6-px");
        let mut p = NdProxy::new(tenant);
        p.upsert(NdProxyEntry {
            remote_pod_ip: v6("fd00:1::5"),
            remote_node_ip: IpAddr::V6(v6("2001:db8::1")),
            local_node_mac: mac(0xAA),
        });
        let answer = p.answer_neighbor_solicit(v6("fd00:1::5")).unwrap();
        assert_eq!(answer, mac(0xAA));
    }

    #[test]
    fn ndproxy_unknown_target_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Proxy.Unknown", "tenant-v6-pxu");
        let p = NdProxy::new(tenant);
        assert!(p.answer_neighbor_solicit(v6("fd00:1::5")).is_none());
    }

    #[test]
    fn ndproxy_remove_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Proxy.Remove", "tenant-v6-pxr");
        let mut p = NdProxy::new(tenant);
        p.upsert(NdProxyEntry {
            remote_pod_ip: v6("fd00:1::5"),
            remote_node_ip: IpAddr::V6(v6("2001:db8::1")),
            local_node_mac: mac(0xAA),
        });
        assert!(p.remove(v6("fd00:1::5")));
        assert!(p.answer_neighbor_solicit(v6("fd00:1::5")).is_none());
    }

    #[test]
    fn ndproxy_count_tracks_upserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Proxy.Count", "tenant-v6-pxc");
        let mut p = NdProxy::new(tenant);
        for i in 0..5u8 {
            p.upsert(NdProxyEntry {
                remote_pod_ip: v6(&format!("fd00:1::{i}")),
                remote_node_ip: IpAddr::V6(v6("2001:db8::1")),
                local_node_mac: mac(i),
            });
        }
        assert_eq!(p.entry_count(), 5);
    }

    #[test]
    fn ndproxy_upsert_replaces_existing() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Proxy.Replace", "tenant-v6-pxr2");
        let mut p = NdProxy::new(tenant);
        p.upsert(NdProxyEntry {
            remote_pod_ip: v6("fd00:1::5"),
            remote_node_ip: IpAddr::V6(v6("2001:db8::1")),
            local_node_mac: mac(0xAA),
        });
        p.upsert(NdProxyEntry {
            remote_pod_ip: v6("fd00:1::5"),
            remote_node_ip: IpAddr::V6(v6("2001:db8::2")),
            local_node_mac: mac(0xBB),
        });
        assert_eq!(p.entry_count(), 1);
        assert_eq!(p.answer_neighbor_solicit(v6("fd00:1::5")), Some(mac(0xBB)));
    }

    // ── SLAAC ────────────────────────────────────────────────────────────────

    #[test]
    fn slaac_address_combines_prefix_and_suffix() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipv6/slaac.go", "Generate", "tenant-v6-slaac");
        let addr = slaac_address("fd00:1::/64", 0xAB).unwrap();
        let s = addr.to_string();
        assert!(s.starts_with("fd00:1::"));
        assert!(s.ends_with("ab"));
    }

    #[test]
    fn slaac_address_distinct_suffixes_distinct_addresses() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipv6/slaac.go", "Generate.Distinct", "tenant-v6-slaacd");
        let a = slaac_address("fd00:1::/64", 1).unwrap();
        let b = slaac_address("fd00:1::/64", 2).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn slaac_with_too_long_prefix_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipv6/slaac.go", "Generate.BadPrefix", "tenant-v6-slaacbad");
        let err = slaac_address("fd00:1::/96", 1).unwrap_err();
        assert!(matches!(err, Ipv6Error::BadCidr(_)));
    }

    #[test]
    fn slaac_with_invalid_cidr_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipv6/slaac.go", "Generate.BadCidr", "tenant-v6-slaacinv");
        let err = slaac_address("not-a-cidr", 1).unwrap_err();
        assert!(matches!(err, Ipv6Error::BadCidr(_)));
    }

    #[test]
    fn slaac_with_v4_cidr_rejected() {
        let (_c, _t) = cilium_test_ctx!("pkg/ipam/ipv6/slaac.go", "Generate.V4Cidr", "tenant-v6-slaacv4");
        let err = slaac_address("10.0.0.0/24", 1).unwrap_err();
        assert!(matches!(err, Ipv6Error::BadCidr(_)));
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn neigh_cache_count_tracks_confirms() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Cache.Count", "tenant-v6-nc");
        let mut c = NeighborCache::new(tenant);
        for i in 1..=5u8 {
            c.confirm(v6(&format!("fd00::{i}")), mac(i), 0);
        }
        assert_eq!(c.len(), 5);
    }

    // ── Serde ────────────────────────────────────────────────────────────────

    #[test]
    fn ndp_type_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/icmp6.h", "ICMPv6.Serde", "tenant-v6-ndserde");
        for t in [
            NdpType::RouterSolicitation,
            NdpType::NeighborSolicitation,
            NdpType::NeighborAdvertisement,
        ] {
            let s = serde_json::to_string(&t).unwrap();
            let back: NdpType = serde_json::from_str(&s).unwrap();
            assert_eq!(back, t);
        }
    }

    #[test]
    fn nud_state_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "NudState.Serde", "tenant-v6-nudserde");
        for s in [NudState::Reachable, NudState::Stale, NudState::Probe, NudState::Failed] {
            let j = serde_json::to_string(&s).unwrap();
            let back: NudState = serde_json::from_str(&j).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn nd_proxy_entry_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Proxy.Serde", "tenant-v6-pxserde");
        let e = NdProxyEntry {
            remote_pod_ip: v6("fd00:1::5"),
            remote_node_ip: IpAddr::V6(v6("2001:db8::1")),
            local_node_mac: mac(0xAA),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: NdProxyEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn neighbor_entry_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/linux/ipv6/ndp.go", "Entry.Serde", "tenant-v6-eserde");
        let e = NeighborEntry {
            ip: v6("fd00::1"), mac: mac(1),
            state: NudState::Reachable, last_update_ns: 100,
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: NeighborEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back, e);
    }
}
