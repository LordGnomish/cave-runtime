// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Connection tracking — Cilium's per-flow state table.
//!
//! Mirrors `bpf/lib/conntrack.h` (the kernel-side state machine) and
//! `pkg/maps/ctmap` (user-space metadata). The real Cilium implementation
//! lives in eBPF C; we port the *semantics* (5-tuple keying, TCP state
//! transitions, idle expiry, LRU eviction, rev-nat linkage) into Rust.
//!
//! Semantics (faithful to upstream):
//!
//! * Entries keyed by `(src_ip, src_port, dst_ip, dst_port, proto)` plus
//!   a [`Direction`] (mirrors `__ct_lookup4` distinguishing egress vs
//!   ingress lookups).
//! * TCP entries advance through a small state machine — `SynSent →
//!   SynRecv → Established → FinWait → TimeWait → Closed`. RST jumps
//!   straight to `Closed`.
//! * UDP and ICMP have no state machine; presence in the table means
//!   "in-flight"; expiry is purely time-based.
//! * Each state has an upstream-defined idle timeout (`CT_*_TIMEOUT`).
//! * The table has a fixed capacity; the oldest entry by `last_seen` is
//!   evicted on overflow (mirrors the kernel `BPF_MAP_TYPE_LRU_HASH`).
//! * `rev_nat_index` links to a NAT mapping in [`super::nat`] so the
//!   datapath can rewrite reply packets without re-running the LB hash.

use crate::cilium::policy::L4Protocol;
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Egress,
    Ingress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Tuple {
    pub src_ip: IpAddr,
    pub src_port: u16,
    pub dst_ip: IpAddr,
    pub dst_port: u16,
    pub protocol: L4Protocol,
}

impl Tuple {
    pub fn new(src_ip: IpAddr, src_port: u16, dst_ip: IpAddr, dst_port: u16, protocol: L4Protocol) -> Self {
        Self { src_ip, src_port, dst_ip, dst_port, protocol }
    }
    /// The reverse 5-tuple used to look up a reply packet (mirrors the
    /// `__ct_lookup4` second probe).
    pub fn reverse(self) -> Self {
        Self {
            src_ip: self.dst_ip,
            src_port: self.dst_port,
            dst_ip: self.src_ip,
            dst_port: self.src_port,
            protocol: self.protocol,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TcpState {
    Closed,
    SynSent,
    SynRecv,
    Established,
    FinWait,
    TimeWait,
}

/// Upstream timeout constants from `bpf/lib/conntrack.h`. We model them in
/// seconds and let the test harness pass in a wall-clock; the production
/// eBPF code uses jiffies / ktime_ns, but the *value* is what matters.
pub const CT_SYN_SENT_TIMEOUT: u64 = 60;
pub const CT_SYN_RECV_TIMEOUT: u64 = 60;
pub const CT_ESTABLISHED_TIMEOUT: u64 = 5 * 24 * 60 * 60; // 5 days
pub const CT_FIN_WAIT_TIMEOUT: u64 = 30;
pub const CT_TIME_WAIT_TIMEOUT: u64 = 60;
pub const CT_UDP_TIMEOUT: u64 = 30;
pub const CT_ICMP_TIMEOUT: u64 = 30;

#[derive(Debug, Clone, Copy)]
pub enum TcpFlag {
    Syn,
    SynAck,
    Ack,
    Fin,
    Rst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CtEntry {
    pub tcp_state: Option<TcpState>,
    pub last_seen: u64,
    pub created: u64,
    pub packets: u64,
    pub bytes: u64,
    pub rev_nat_index: Option<u16>,
    pub direction: Direction,
}

impl CtEntry {
    fn timeout(&self, proto: L4Protocol) -> u64 {
        match proto {
            L4Protocol::TCP => match self.tcp_state.unwrap_or(TcpState::Closed) {
                TcpState::SynSent => CT_SYN_SENT_TIMEOUT,
                TcpState::SynRecv => CT_SYN_RECV_TIMEOUT,
                TcpState::Established => CT_ESTABLISHED_TIMEOUT,
                TcpState::FinWait => CT_FIN_WAIT_TIMEOUT,
                TcpState::TimeWait => CT_TIME_WAIT_TIMEOUT,
                TcpState::Closed => 0,
            },
            L4Protocol::UDP => CT_UDP_TIMEOUT,
            L4Protocol::ICMP => CT_ICMP_TIMEOUT,
            L4Protocol::SCTP => CT_ESTABLISHED_TIMEOUT,
            L4Protocol::Any => CT_UDP_TIMEOUT,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CtError {
    #[error("tenant {tenant} cannot mutate ct table owned by another tenant")]
    TenantDenied { tenant: TenantId },
    #[error("ct table is empty")]
    Empty,
}

#[derive(Debug)]
pub struct ConntrackTable {
    pub tenant: TenantId,
    pub capacity: usize,
    entries: HashMap<(Tuple, Direction), CtEntry>,
}

impl ConntrackTable {
    pub fn new(tenant: TenantId, capacity: usize) -> Self {
        Self { tenant, capacity, entries: HashMap::new() }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert or refresh an entry. If the table is at capacity, the entry
    /// with the smallest `last_seen` is evicted. Returns the new entry.
    pub fn upsert(
        &mut self,
        tuple: Tuple,
        dir: Direction,
        now: u64,
        bytes: u64,
        rev_nat: Option<u16>,
    ) -> CtEntry {
        let key = (tuple, dir);
        let existing = self.entries.get(&key).copied();
        let entry = match existing {
            Some(mut e) => {
                e.last_seen = now;
                e.packets += 1;
                e.bytes += bytes;
                if rev_nat.is_some() {
                    e.rev_nat_index = rev_nat;
                }
                e
            }
            None => {
                if self.entries.len() >= self.capacity {
                    self.evict_oldest();
                }
                CtEntry {
                    tcp_state: if matches!(tuple.protocol, L4Protocol::TCP) {
                        Some(TcpState::SynSent)
                    } else {
                        None
                    },
                    last_seen: now,
                    created: now,
                    packets: 1,
                    bytes,
                    rev_nat_index: rev_nat,
                    direction: dir,
                }
            }
        };
        self.entries.insert(key, entry);
        entry
    }

    /// Apply a TCP flag transition. Mirrors `__ct_update_tcp_state` in
    /// upstream `bpf/lib/conntrack.h`.
    pub fn apply_tcp_flag(
        &mut self,
        tuple: Tuple,
        dir: Direction,
        flag: TcpFlag,
        now: u64,
    ) -> Option<CtEntry> {
        let key = (tuple, dir);
        let mut e = *self.entries.get(&key)?;
        let cur = e.tcp_state.unwrap_or(TcpState::Closed);
        let next = match (cur, flag) {
            (_, TcpFlag::Rst) => TcpState::Closed,
            (TcpState::Closed | TcpState::SynSent, TcpFlag::Syn) => TcpState::SynSent,
            (TcpState::SynSent, TcpFlag::SynAck) => TcpState::SynRecv,
            (TcpState::SynRecv | TcpState::SynSent, TcpFlag::Ack) => TcpState::Established,
            (TcpState::Established, TcpFlag::Ack) => TcpState::Established,
            (TcpState::Established | TcpState::SynRecv, TcpFlag::Fin) => TcpState::FinWait,
            (TcpState::FinWait, TcpFlag::Ack) => TcpState::TimeWait,
            (s, _) => s, // no-op transition
        };
        e.tcp_state = Some(next);
        e.last_seen = now;
        self.entries.insert(key, e);
        Some(e)
    }

    /// Lookup an entry by tuple + direction. Mirrors `ct_lookup4` first probe.
    pub fn lookup(&self, tuple: Tuple, dir: Direction) -> Option<CtEntry> {
        self.entries.get(&(tuple, dir)).copied()
    }

    /// Look up a reply packet by reversing the 5-tuple. Mirrors the
    /// second probe in `ct_lookup4` that handles DNAT'd return traffic.
    pub fn lookup_reverse(&self, tuple: Tuple, dir: Direction) -> Option<CtEntry> {
        let original_dir = match dir {
            Direction::Egress => Direction::Ingress,
            Direction::Ingress => Direction::Egress,
        };
        self.entries.get(&(tuple.reverse(), original_dir)).copied()
    }

    /// Remove all entries whose `last_seen + timeout(state) <= now`.
    /// Returns the count of purged entries. Mirrors `ctmap.GC`.
    pub fn purge_idle(&mut self, now: u64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|(t, _), e| {
            let to = e.timeout(t.protocol);
            now.saturating_sub(e.last_seen) < to
        });
        before - self.entries.len()
    }

    fn evict_oldest(&mut self) {
        if let Some((&key, _)) = self.entries.iter().min_by_key(|(_, e)| e.last_seen) {
            self.entries.remove(&key);
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("bpf/lib/conntrack.h", "__ct_lookup4");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }
    fn t(a: u8, b: u8, c: u8, d: u8, sp: u16, dst: (u8, u8, u8, u8), dp: u16, p: L4Protocol) -> Tuple {
        Tuple::new(ip(a, b, c, d), sp, ip(dst.0, dst.1, dst.2, dst.3), dp, p)
    }

    // ── insert / lookup ──────────────────────────────────────────────────────

    #[test]
    fn ct_insert_then_lookup_round_trips() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_lookup4", "tenant-ct-rt");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 33333, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 1000, 64, None);
        let e = ct.lookup(tup, Direction::Egress).unwrap();
        assert_eq!(e.packets, 1);
        assert_eq!(e.bytes, 64);
    }

    #[test]
    fn ct_lookup_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "ct_lookup4", "tenant-ct-miss");
        let ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1, (10, 0, 0, 2), 2, L4Protocol::TCP);
        assert!(ct.lookup(tup, Direction::Egress).is_none());
    }

    #[test]
    fn ct_unique_5tuples_create_distinct_entries() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "ct_lookup4", "tenant-ct-uniq");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let a = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        let b = t(10, 0, 0, 1, 1235, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(a, Direction::Egress, 100, 1, None);
        ct.upsert(b, Direction::Egress, 100, 1, None);
        assert_eq!(ct.len(), 2);
    }

    #[test]
    fn ct_upsert_existing_increments_packets_and_bytes() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update", "tenant-ct-incr");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 64, None);
        ct.upsert(tup, Direction::Egress, 200, 128, None);
        let e = ct.lookup(tup, Direction::Egress).unwrap();
        assert_eq!(e.packets, 2);
        assert_eq!(e.bytes, 192);
        assert_eq!(e.last_seen, 200);
    }

    #[test]
    fn ct_directional_lookup_distinguishes_egress_and_ingress() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_lookup4", "tenant-ct-dir");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        assert!(ct.lookup(tup, Direction::Egress).is_some());
        assert!(ct.lookup(tup, Direction::Ingress).is_none());
    }

    #[test]
    fn ct_reverse_lookup_finds_dnat_reply() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "ct_lookup4_reverse", "tenant-ct-rev");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, Some(7));
        // Reply packet hits the box from the backend → reversed 5-tuple, ingress.
        let reply = t(10, 96, 0, 1, 80, (10, 0, 0, 1), 1234, L4Protocol::TCP);
        let e = ct.lookup_reverse(reply, Direction::Ingress).unwrap();
        assert_eq!(e.rev_nat_index, Some(7));
    }

    // ── TCP state machine ────────────────────────────────────────────────────

    #[test]
    fn ct_tcp_first_packet_creates_syn_sent() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_create_tcp", "tenant-ct-syn");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        let e = ct.upsert(tup, Direction::Egress, 100, 1, None);
        assert_eq!(e.tcp_state, Some(TcpState::SynSent));
    }

    #[test]
    fn ct_tcp_synack_advances_to_syn_recv() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update_tcp_state", "tenant-ct-synack");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        let e = ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::SynAck, 110).unwrap();
        assert_eq!(e.tcp_state, Some(TcpState::SynRecv));
    }

    #[test]
    fn ct_tcp_ack_after_syn_recv_advances_to_established() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update_tcp_state", "tenant-ct-est");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::SynAck, 110);
        let e = ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Ack, 111).unwrap();
        assert_eq!(e.tcp_state, Some(TcpState::Established));
    }

    #[test]
    fn ct_tcp_fin_advances_to_fin_wait() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update_tcp_state", "tenant-ct-fin");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::SynAck, 110);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Ack, 111);
        let e = ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Fin, 200).unwrap();
        assert_eq!(e.tcp_state, Some(TcpState::FinWait));
    }

    #[test]
    fn ct_tcp_fin_then_ack_advances_to_time_wait() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update_tcp_state", "tenant-ct-tw");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::SynAck, 110);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Ack, 111);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Fin, 200);
        let e = ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Ack, 201).unwrap();
        assert_eq!(e.tcp_state, Some(TcpState::TimeWait));
    }

    #[test]
    fn ct_tcp_rst_jumps_to_closed_from_any_state() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update_tcp_state", "tenant-ct-rst");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::SynAck, 110);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Ack, 111);
        let e = ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Rst, 200).unwrap();
        assert_eq!(e.tcp_state, Some(TcpState::Closed));
    }

    #[test]
    fn ct_apply_flag_to_unknown_returns_none() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update_tcp_state", "tenant-ct-flagmiss");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        assert!(ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Syn, 100).is_none());
    }

    #[test]
    fn ct_udp_entry_has_no_tcp_state() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_create_udp", "tenant-ct-udp");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 5353, (10, 96, 0, 10), 53, L4Protocol::UDP);
        let e = ct.upsert(tup, Direction::Egress, 100, 1, None);
        assert_eq!(e.tcp_state, None);
    }

    #[test]
    fn ct_icmp_entry_has_no_tcp_state() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_create_icmp", "tenant-ct-icmp");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 0, (10, 0, 0, 2), 0, L4Protocol::ICMP);
        let e = ct.upsert(tup, Direction::Egress, 100, 1, None);
        assert_eq!(e.tcp_state, None);
    }

    // ── expiry / GC ──────────────────────────────────────────────────────────

    #[test]
    fn ct_purge_removes_idle_udp_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/ctmap/ctmap.go", "GC", "tenant-ct-gc-udp");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 5353, (10, 96, 0, 10), 53, L4Protocol::UDP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        let removed = ct.purge_idle(100 + CT_UDP_TIMEOUT);
        assert_eq!(removed, 1);
        assert!(ct.is_empty());
    }

    #[test]
    fn ct_purge_keeps_recent_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/ctmap/ctmap.go", "GC", "tenant-ct-gc-recent");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 5353, (10, 96, 0, 10), 53, L4Protocol::UDP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        let removed = ct.purge_idle(100 + 1);
        assert_eq!(removed, 0);
        assert_eq!(ct.len(), 1);
    }

    #[test]
    fn ct_established_tcp_keeps_long_ttl() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/ctmap/ctmap.go", "GC.TCPEstablished", "tenant-ct-gc-est");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::SynAck, 100);
        ct.apply_tcp_flag(tup, Direction::Egress, TcpFlag::Ack, 100);
        let removed = ct.purge_idle(100 + CT_FIN_WAIT_TIMEOUT * 100);
        assert_eq!(removed, 0); // established TTL is days
    }

    #[test]
    fn ct_lru_evicts_oldest_when_at_capacity() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "ct_evict", "tenant-ct-lru");
        let mut ct = ConntrackTable::new(tenant, 2);
        let a = t(10, 0, 0, 1, 1, (10, 96, 0, 1), 80, L4Protocol::TCP);
        let b = t(10, 0, 0, 1, 2, (10, 96, 0, 1), 80, L4Protocol::TCP);
        let c = t(10, 0, 0, 1, 3, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(a, Direction::Egress, 100, 1, None);
        ct.upsert(b, Direction::Egress, 200, 1, None);
        ct.upsert(c, Direction::Egress, 300, 1, None);
        assert_eq!(ct.len(), 2);
        // a was the oldest, should be evicted.
        assert!(ct.lookup(a, Direction::Egress).is_none());
        assert!(ct.lookup(c, Direction::Egress).is_some());
    }

    #[test]
    fn ct_rev_nat_index_persists_on_create() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_create.rev_nat_index", "tenant-ct-rni");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, Some(42));
        assert_eq!(ct.lookup(tup, Direction::Egress).unwrap().rev_nat_index, Some(42));
    }

    #[test]
    fn ct_rev_nat_index_overwrites_on_subsequent_upsert() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/conntrack.h", "__ct_update.rev_nat_index", "tenant-ct-rni-up");
        let mut ct = ConntrackTable::new(tenant, 1024);
        let tup = t(10, 0, 0, 1, 1234, (10, 96, 0, 1), 80, L4Protocol::TCP);
        ct.upsert(tup, Direction::Egress, 100, 1, None);
        ct.upsert(tup, Direction::Egress, 200, 1, Some(7));
        assert_eq!(ct.lookup(tup, Direction::Egress).unwrap().rev_nat_index, Some(7));
    }

    #[test]
    fn ct_purge_returns_count_of_removed() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maps/ctmap/ctmap.go", "GC", "tenant-ct-gc-count");
        let mut ct = ConntrackTable::new(tenant, 1024);
        for i in 0..5 {
            let tup = t(10, 0, 0, 1, 5353 + i, (10, 96, 0, 10), 53, L4Protocol::UDP);
            ct.upsert(tup, Direction::Egress, 100, 1, None);
        }
        let removed = ct.purge_idle(100 + CT_UDP_TIMEOUT + 1);
        assert_eq!(removed, 5);
    }
}
