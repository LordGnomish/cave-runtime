// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! NAT table — SNAT (masquerade) and DNAT (service backend rewrite).
//!
//! Mirrors `pkg/datapath/linux/nat/nat.go` and the BPF NAT helpers in
//! `bpf/lib/nat.h`. Keys SNAT entries by the *original* 5-tuple → the
//! rewritten one, and DNAT entries by service identity → backend.
//!
//! Semantics (faithful to upstream):
//!
//! * SNAT allocates a free source port from a configurable range
//!   (default 32768..=65535, mirrors `EphemeralPortMin/Max`).
//! * Re-allocating with the same original 5-tuple is idempotent — the
//!   same source port is returned (mirrors the `__nat_create_v4` pre-flight
//!   that scans the existing entry first).
//! * Releasing a mapping frees the port for re-use.
//! * DNAT entries use a 16-bit `rev_nat_index` matching the conntrack
//!   field; the value is a `(backend_ip, backend_port)` pair.
//! * Conflict avoidance: if the chosen port is already taken by a
//!   *different* original tuple, the allocator retries.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

pub const EPHEMERAL_PORT_MIN: u16 = 32768;
pub const EPHEMERAL_PORT_MAX: u16 = 65535;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SnatKey {
    pub src_ip: IpAddr,
    pub src_port: u16,
    pub dst_ip: IpAddr,
    pub dst_port: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnatEntry {
    pub new_src_ip: IpAddr,
    pub new_src_port: u16,
    pub created: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DnatEntry {
    pub backend_ip: IpAddr,
    pub backend_port: u16,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum NatError {
    #[error("SNAT port range exhausted (no free port for {src})")]
    PortExhausted { src: IpAddr },
    #[error("tenant {tenant} cannot mutate NAT table owned by another tenant")]
    TenantDenied { tenant: TenantId },
    #[error("rev_nat_index overflow (16-bit, allocated {0})")]
    RevNatExhausted(u32),
}

/// SNAT (masquerade) table. Allocates a `(node_ip, src_port)` for each
/// outbound flow.
#[derive(Debug)]
pub struct SnatTable {
    pub tenant: TenantId,
    pub node_ip: IpAddr,
    forward: HashMap<SnatKey, SnatEntry>,
    /// Reverse index: (new_src_ip, new_src_port, dst_ip, dst_port) → original key.
    reverse: HashMap<SnatKey, SnatKey>,
}

impl SnatTable {
    pub fn new(tenant: TenantId, node_ip: IpAddr) -> Self {
        Self { tenant, node_ip, forward: HashMap::new(), reverse: HashMap::new() }
    }

    /// Allocate or look up the SNAT mapping for the given original 5-tuple.
    /// Idempotent for the same 5-tuple.
    pub fn allocate(&mut self, key: SnatKey, now: u64) -> Result<SnatEntry, NatError> {
        if let Some(&entry) = self.forward.get(&key) {
            return Ok(entry);
        }
        // Probe ports starting from a hash of the source port, walking the
        // ephemeral range. Mirrors `__nat_select_port_v4`.
        let start = EPHEMERAL_PORT_MIN
            .wrapping_add(key.src_port % (EPHEMERAL_PORT_MAX - EPHEMERAL_PORT_MIN + 1));
        let span = (EPHEMERAL_PORT_MAX as u32 - EPHEMERAL_PORT_MIN as u32 + 1) as u16;
        for off in 0..span {
            let candidate = EPHEMERAL_PORT_MIN
                + (((start - EPHEMERAL_PORT_MIN) as u32 + off as u32) % span as u32) as u16;
            let rev_key = SnatKey {
                src_ip: self.node_ip,
                src_port: candidate,
                dst_ip: key.dst_ip,
                dst_port: key.dst_port,
            };
            if !self.reverse.contains_key(&rev_key) {
                let entry = SnatEntry { new_src_ip: self.node_ip, new_src_port: candidate, created: now };
                self.forward.insert(key, entry);
                self.reverse.insert(rev_key, key);
                return Ok(entry);
            }
        }
        Err(NatError::PortExhausted { src: key.src_ip })
    }

    pub fn lookup(&self, key: &SnatKey) -> Option<SnatEntry> {
        self.forward.get(key).copied()
    }

    /// Reverse lookup: given a reply packet's outer-tuple, find the original
    /// inner key.
    pub fn lookup_reverse(&self, key: &SnatKey) -> Option<SnatKey> {
        self.reverse.get(key).copied()
    }

    pub fn release(&mut self, key: &SnatKey) -> bool {
        if let Some(entry) = self.forward.remove(key) {
            let rev_key = SnatKey {
                src_ip: entry.new_src_ip,
                src_port: entry.new_src_port,
                dst_ip: key.dst_ip,
                dst_port: key.dst_port,
            };
            self.reverse.remove(&rev_key);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.forward.len()
    }
}

/// DNAT table — service-IP → backend mapping, indexed by 16-bit rev_nat_index.
#[derive(Debug)]
pub struct DnatTable {
    pub tenant: TenantId,
    next_index: u32,
    by_index: HashMap<u16, DnatEntry>,
    /// Service slot key → rev_nat_index (so re-registering the same backend is idempotent).
    by_backend: HashMap<DnatEntry, u16>,
}

impl DnatTable {
    pub fn new(tenant: TenantId) -> Self {
        Self { tenant, next_index: 1, by_index: HashMap::new(), by_backend: HashMap::new() }
    }

    pub fn install(&mut self, entry: DnatEntry) -> Result<u16, NatError> {
        if let Some(&idx) = self.by_backend.get(&entry) {
            return Ok(idx);
        }
        if self.next_index > u16::MAX as u32 {
            return Err(NatError::RevNatExhausted(self.next_index - 1));
        }
        let idx = self.next_index as u16;
        self.next_index += 1;
        self.by_index.insert(idx, entry);
        self.by_backend.insert(entry, idx);
        Ok(idx)
    }

    pub fn lookup(&self, idx: u16) -> Option<DnatEntry> {
        self.by_index.get(&idx).copied()
    }

    pub fn remove(&mut self, idx: u16) -> bool {
        if let Some(entry) = self.by_index.remove(&idx) {
            self.by_backend.remove(&entry);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.by_index.len()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/datapath/linux/nat/nat.go", "Map");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn key(s: (u8, u8, u8, u8), sp: u16, d: (u8, u8, u8, u8), dp: u16) -> SnatKey {
        SnatKey { src_ip: ip(s.0, s.1, s.2, s.3), src_port: sp, dst_ip: ip(d.0, d.1, d.2, d.3), dst_port: dp }
    }

    // ── SNAT ─────────────────────────────────────────────────────────────────

    #[test]
    fn snat_allocate_returns_new_port_in_ephemeral_range() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.AllocV4", "tenant-nat-alloc");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        let k = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        let e = nat.allocate(k, 100).unwrap();
        assert_eq!(e.new_src_ip, ip(192, 168, 1, 1));
        assert!((EPHEMERAL_PORT_MIN..=EPHEMERAL_PORT_MAX).contains(&e.new_src_port));
    }

    #[test]
    fn snat_allocate_is_idempotent_for_same_5tuple() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.AllocV4", "tenant-nat-idem");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        let k = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        let a = nat.allocate(k, 100).unwrap();
        let b = nat.allocate(k, 200).unwrap();
        assert_eq!(a.new_src_port, b.new_src_port);
    }

    #[test]
    fn snat_allocate_two_distinct_5tuples_get_different_ports_when_collision() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.AllocV4.Collision", "tenant-nat-coll");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        // Two flows that hash to the same starting port (same src_port, same dst).
        let k1 = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        let k2 = key((10, 0, 0, 2), 1234, (1, 1, 1, 1), 80);
        let a = nat.allocate(k1, 100).unwrap();
        let b = nat.allocate(k2, 100).unwrap();
        assert_ne!(a.new_src_port, b.new_src_port);
    }

    #[test]
    fn snat_lookup_returns_existing_entry() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.LookupV4", "tenant-nat-lk");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        let k = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        let a = nat.allocate(k, 100).unwrap();
        assert_eq!(nat.lookup(&k), Some(a));
    }

    #[test]
    fn snat_lookup_reverse_finds_original_key() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.LookupV4Reverse", "tenant-nat-revl");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        let k = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        let a = nat.allocate(k, 100).unwrap();
        let rev = SnatKey {
            src_ip: a.new_src_ip,
            src_port: a.new_src_port,
            dst_ip: k.dst_ip,
            dst_port: k.dst_port,
        };
        assert_eq!(nat.lookup_reverse(&rev), Some(k));
    }

    #[test]
    fn snat_release_frees_mapping() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.DeleteV4", "tenant-nat-rel");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        let k = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        nat.allocate(k, 100).unwrap();
        assert!(nat.release(&k));
        assert_eq!(nat.lookup(&k), None);
    }

    #[test]
    fn snat_release_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.DeleteV4", "tenant-nat-rel-unk");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        let k = key((10, 0, 0, 1), 1234, (1, 1, 1, 1), 80);
        assert!(!nat.release(&k));
    }

    #[test]
    fn snat_table_len_tracks_allocations() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.Len", "tenant-nat-len");
        let mut nat = SnatTable::new(tenant, ip(192, 168, 1, 1));
        for i in 0..10u16 {
            let k = key((10, 0, 0, 1), 1000 + i, (1, 1, 1, 1), 80);
            nat.allocate(k, 100).unwrap();
        }
        assert_eq!(nat.len(), 10);
    }

    // ── DNAT ─────────────────────────────────────────────────────────────────

    #[test]
    fn dnat_install_returns_monotonic_index() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/nat.h", "ct_create4_with_rev_nat", "tenant-dnat-mono");
        let mut dnat = DnatTable::new(tenant);
        let a = dnat.install(DnatEntry { backend_ip: ip(10, 0, 1, 1), backend_port: 8080 }).unwrap();
        let b = dnat.install(DnatEntry { backend_ip: ip(10, 0, 1, 2), backend_port: 8080 }).unwrap();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn dnat_install_same_backend_is_idempotent() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/nat.h", "ct_create4_with_rev_nat", "tenant-dnat-idem");
        let mut dnat = DnatTable::new(tenant);
        let e = DnatEntry { backend_ip: ip(10, 0, 1, 1), backend_port: 8080 };
        let a = dnat.install(e).unwrap();
        let b = dnat.install(e).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn dnat_lookup_round_trip() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/nat.h", "snat_v4_track_lookup", "tenant-dnat-rt");
        let mut dnat = DnatTable::new(tenant);
        let e = DnatEntry { backend_ip: ip(10, 0, 1, 1), backend_port: 8080 };
        let idx = dnat.install(e).unwrap();
        assert_eq!(dnat.lookup(idx), Some(e));
    }

    #[test]
    fn dnat_remove_drops_entry() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/nat.h", "snat_v4_delete", "tenant-dnat-rm");
        let mut dnat = DnatTable::new(tenant);
        let e = DnatEntry { backend_ip: ip(10, 0, 1, 1), backend_port: 8080 };
        let idx = dnat.install(e).unwrap();
        assert!(dnat.remove(idx));
        assert_eq!(dnat.lookup(idx), None);
    }

    #[test]
    fn dnat_remove_unknown_returns_false() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/nat.h", "snat_v4_delete", "tenant-dnat-rmunk");
        let mut dnat = DnatTable::new(tenant);
        assert!(!dnat.remove(42));
    }

    #[test]
    fn dnat_len_tracks_inserts() {
        let (_c, tenant) = cilium_test_ctx!("pkg/datapath/linux/nat/nat.go", "Map.Len", "tenant-dnat-len");
        let mut dnat = DnatTable::new(tenant);
        for p in 8000..8005u16 {
            dnat.install(DnatEntry { backend_ip: ip(10, 0, 1, 1), backend_port: p }).unwrap();
        }
        assert_eq!(dnat.len(), 5);
    }
}
