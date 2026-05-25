// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BPF map abstractions — userspace models of the kernel-side maps.
//!
//! Mirrors the various map definitions under `bpf/` and the Go-side
//! wrappers in `pkg/maps/`. The real kernel maps are eBPF-backed; we
//! model only the *behavioural* surface (key/value semantics, capacity
//! limits, LRU eviction, longest-prefix-match for LPM tries).
//!
//! Map flavours (faithful to upstream):
//!
//! * [`HashMapBpf`] — `BPF_MAP_TYPE_HASH`. Fixed capacity; insert beyond
//!   capacity returns `MapFull`.
//! * [`LruMap`] — `BPF_MAP_TYPE_LRU_HASH`. Same as `HashMapBpf` but
//!   evicts the least-recently-touched entry on overflow. Lookup
//!   promotes (mirrors kernel reuseport).
//! * [`ArrayMap`] — `BPF_MAP_TYPE_ARRAY`. Fixed-size flat array indexed
//!   by `u32`; out-of-bounds → error.
//! * [`LpmTrie`] — `BPF_MAP_TYPE_LPM_TRIE`. Keyed by CIDR; lookup
//!   returns the value of the *longest* matching prefix.

use crate::cilium::types::{Cite, TenantId};
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::Hash;
use std::net::IpAddr;
use std::str::FromStr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MapError {
    #[error("map at capacity ({0})")]
    MapFull(usize),
    #[error("array index {0} out of bounds (capacity {1})")]
    OutOfBounds(u32, usize),
    #[error("invalid CIDR `{0}`")]
    BadCidr(String),
    #[error("tenant {tenant} cannot mutate map owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

// ── Hash map ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct HashMapBpf<K: Eq + Hash + Clone, V: Clone> {
    pub capacity: usize,
    inner: HashMap<K, V>,
}

impl<K: Eq + Hash + Clone + Ord, V: Clone> HashMapBpf<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: HashMap::new(),
        }
    }
    pub fn insert(&mut self, k: K, v: V) -> Result<(), MapError> {
        if self.inner.contains_key(&k) {
            self.inner.insert(k, v);
            return Ok(());
        }
        if self.inner.len() >= self.capacity {
            return Err(MapError::MapFull(self.capacity));
        }
        self.inner.insert(k, v);
        Ok(())
    }
    pub fn lookup(&self, k: &K) -> Option<&V> {
        self.inner.get(k)
    }
    pub fn remove(&mut self, k: &K) -> bool {
        self.inner.remove(k).is_some()
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
    pub fn clear(&mut self) {
        self.inner.clear();
    }
    /// Iter sorted by key for deterministic dumps.
    pub fn iter_sorted(&self) -> Vec<(K, V)> {
        let mut v: Vec<_> = self
            .inner
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

// ── LRU map ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LruMap<K: Eq + Hash + Clone, V: Clone> {
    pub capacity: usize,
    inner: HashMap<K, V>,
    /// Most-recent at the back, oldest at the front.
    order: VecDeque<K>,
}

impl<K: Eq + Hash + Clone, V: Clone> LruMap<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            inner: HashMap::new(),
            order: VecDeque::new(),
        }
    }
    pub fn insert(&mut self, k: K, v: V) -> Option<(K, V)> {
        let mut evicted = None;
        if !self.inner.contains_key(&k) {
            if self.inner.len() >= self.capacity {
                if let Some(oldest) = self.order.pop_front() {
                    if let Some(v) = self.inner.remove(&oldest) {
                        evicted = Some((oldest, v));
                    }
                }
            }
            self.order.push_back(k.clone());
        } else {
            // Promote.
            if let Some(pos) = self.order.iter().position(|x| x == &k) {
                self.order.remove(pos);
            }
            self.order.push_back(k.clone());
        }
        self.inner.insert(k, v);
        evicted
    }
    /// Lookup promotes (kernel LRU semantics).
    pub fn lookup(&mut self, k: &K) -> Option<V> {
        let v = self.inner.get(k)?.clone();
        if let Some(pos) = self.order.iter().position(|x| x == k) {
            self.order.remove(pos);
        }
        self.order.push_back(k.clone());
        Some(v)
    }
    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn oldest(&self) -> Option<&K> {
        self.order.front()
    }
}

// ── Array map ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ArrayMap<V: Clone + Default> {
    inner: Vec<V>,
}

impl<V: Clone + Default> ArrayMap<V> {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: vec![V::default(); capacity],
        }
    }
    pub fn lookup(&self, idx: u32) -> Result<&V, MapError> {
        self.inner
            .get(idx as usize)
            .ok_or(MapError::OutOfBounds(idx, self.inner.len()))
    }
    pub fn write(&mut self, idx: u32, v: V) -> Result<(), MapError> {
        let cap = self.inner.len();
        let slot = self
            .inner
            .get_mut(idx as usize)
            .ok_or(MapError::OutOfBounds(idx, cap))?;
        *slot = v;
        Ok(())
    }
    pub fn capacity(&self) -> usize {
        self.inner.len()
    }
}

// ── LPM trie ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LpmTrie<V: Clone> {
    /// CIDR string → value. Lookup walks all and picks the longest match.
    entries: BTreeMap<String, V>,
}

impl<V: Clone> Default for LpmTrie<V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<V: Clone> LpmTrie<V> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, cidr: impl Into<String>, v: V) -> Result<(), MapError> {
        let cidr = cidr.into();
        IpNet::from_str(&cidr).map_err(|_| MapError::BadCidr(cidr.clone()))?;
        self.entries.insert(cidr, v);
        Ok(())
    }
    pub fn lookup(&self, ip: IpAddr) -> Option<V> {
        let mut best: Option<(u8, V)> = None;
        for (cidr, v) in &self.entries {
            let net = match IpNet::from_str(cidr) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if !net.contains(&ip) {
                continue;
            }
            let plen = net.prefix_len();
            if best.as_ref().map(|(p, _)| plen > *p).unwrap_or(true) {
                best = Some((plen, v.clone()));
            }
        }
        best.map(|(_, v)| v)
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn remove(&mut self, cidr: &str) -> bool {
        self.entries.remove(cidr).is_some()
    }
}

// ── Map registry — the well-known Cilium maps ────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MapId {
    /// `bpf/cilium_endpoints.h` — endpoint id → endpoint metadata.
    Endpoints,
    /// `bpf/cilium_ipcache.h` — IP → identity (for remote pod lookups).
    Ipcache,
    /// `bpf/cilium_policy.h` — (endpoint, identity, port, proto, dir) → verdict.
    Policy,
    /// `bpf/cilium_ct_tcp4.h` — TCP conntrack table.
    CtTcp,
    /// `bpf/cilium_ct_any4.h` — non-TCP conntrack.
    CtAny,
    /// `bpf/cilium_snat_v4_external.h` — SNAT mappings.
    Nat,
    /// `bpf/cilium_lb4_services.h` — service → backend selection.
    Lb,
    /// `bpf/cilium_lb4_backends.h` — backend metadata.
    LbBackends,
    /// `bpf/cilium_lxc.h` — local endpoint → ifindex.
    Lxc,
}

impl MapId {
    pub fn upstream_path(self) -> &'static str {
        match self {
            MapId::Endpoints => "bpf/cilium_endpoints.h",
            MapId::Ipcache => "bpf/cilium_ipcache.h",
            MapId::Policy => "bpf/cilium_policy.h",
            MapId::CtTcp => "bpf/cilium_ct_tcp4.h",
            MapId::CtAny => "bpf/cilium_ct_any4.h",
            MapId::Nat => "bpf/cilium_snat_v4_external.h",
            MapId::Lb => "bpf/cilium_lb4_services.h",
            MapId::LbBackends => "bpf/cilium_lb4_backends.h",
            MapId::Lxc => "bpf/cilium_lxc.h",
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/datapath/maps/maps.go", "MapID");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;
    use std::net::Ipv4Addr;

    fn ip(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    // ── HashMapBpf ───────────────────────────────────────────────────────────

    #[test]
    fn bpfmap_hash_insert_and_lookup_round_trip() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/maps.h", "BPF_MAP_TYPE_HASH", "tenant-bpf-hash");
        let mut m: HashMapBpf<u32, u32> = HashMapBpf::new(4);
        m.insert(1, 100).unwrap();
        assert_eq!(m.lookup(&1), Some(&100));
    }

    #[test]
    fn bpfmap_hash_remove_drops_key() {
        let (_c, _t) =
            cilium_test_ctx!("bpf/lib/maps.h", "bpf_map_delete_elem", "tenant-bpf-hashrm");
        let mut m: HashMapBpf<u32, u32> = HashMapBpf::new(4);
        m.insert(1, 100).unwrap();
        assert!(m.remove(&1));
        assert!(m.lookup(&1).is_none());
    }

    #[test]
    fn bpfmap_hash_full_returns_error_on_new_insert() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_HASH.MapFull",
            "tenant-bpf-hashfull"
        );
        let mut m: HashMapBpf<u32, u32> = HashMapBpf::new(2);
        m.insert(1, 1).unwrap();
        m.insert(2, 2).unwrap();
        let err = m.insert(3, 3).unwrap_err();
        assert_eq!(err, MapError::MapFull(2));
    }

    #[test]
    fn bpfmap_hash_overwrite_existing_key_does_not_count_against_capacity() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_HASH.Update",
            "tenant-bpf-hashov"
        );
        let mut m: HashMapBpf<u32, u32> = HashMapBpf::new(2);
        m.insert(1, 1).unwrap();
        m.insert(1, 100).unwrap();
        assert_eq!(m.lookup(&1), Some(&100));
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn bpfmap_hash_iter_sorted_returns_keys_in_order() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/maps/maps.go", "Iterate", "tenant-bpf-iter");
        let mut m: HashMapBpf<u32, u32> = HashMapBpf::new(8);
        m.insert(3, 30).unwrap();
        m.insert(1, 10).unwrap();
        m.insert(2, 20).unwrap();
        let v = m.iter_sorted();
        let keys: Vec<u32> = v.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![1, 2, 3]);
    }

    #[test]
    fn bpfmap_hash_clear_drops_all() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/maps/maps.go", "DeleteAll", "tenant-bpf-clr");
        let mut m: HashMapBpf<u32, u32> = HashMapBpf::new(8);
        m.insert(1, 1).unwrap();
        m.insert(2, 2).unwrap();
        m.clear();
        assert!(m.is_empty());
    }

    // ── LruMap ───────────────────────────────────────────────────────────────

    #[test]
    fn bpfmap_lru_insert_evicts_oldest_on_overflow() {
        let (_c, _t) =
            cilium_test_ctx!("bpf/lib/maps.h", "BPF_MAP_TYPE_LRU_HASH", "tenant-bpf-lru");
        let mut m: LruMap<u32, u32> = LruMap::new(2);
        m.insert(1, 1);
        m.insert(2, 2);
        let evicted = m.insert(3, 3);
        assert!(evicted.is_some());
        assert_eq!(evicted.unwrap().0, 1);
    }

    #[test]
    fn bpfmap_lru_lookup_promotes_entry() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_LRU_HASH.Lookup",
            "tenant-bpf-lrup"
        );
        let mut m: LruMap<u32, u32> = LruMap::new(2);
        m.insert(1, 1);
        m.insert(2, 2);
        // Promote 1 to most-recent.
        let _ = m.lookup(&1);
        let evicted = m.insert(3, 3);
        // 2 should be evicted now (1 was promoted).
        assert_eq!(evicted.unwrap().0, 2);
    }

    #[test]
    fn bpfmap_lru_oldest_returns_front_of_order() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_LRU_HASH.Oldest",
            "tenant-bpf-lruo"
        );
        let mut m: LruMap<u32, u32> = LruMap::new(4);
        m.insert(10, 1);
        m.insert(20, 2);
        assert_eq!(m.oldest(), Some(&10));
    }

    // ── ArrayMap ─────────────────────────────────────────────────────────────

    #[test]
    fn bpfmap_array_indexed_lookup_returns_value() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/maps.h", "BPF_MAP_TYPE_ARRAY", "tenant-bpf-arr");
        let mut a: ArrayMap<u32> = ArrayMap::new(8);
        a.write(3, 42).unwrap();
        assert_eq!(a.lookup(3).unwrap(), &42);
    }

    #[test]
    fn bpfmap_array_out_of_bounds_returns_error() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_ARRAY.Bounds",
            "tenant-bpf-arr-oob"
        );
        let a: ArrayMap<u32> = ArrayMap::new(4);
        let err = a.lookup(99).unwrap_err();
        assert_eq!(err, MapError::OutOfBounds(99, 4));
    }

    #[test]
    fn bpfmap_array_capacity_matches_constructor() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_ARRAY.Capacity",
            "tenant-bpf-arr-cap"
        );
        let a: ArrayMap<u8> = ArrayMap::new(64);
        assert_eq!(a.capacity(), 64);
    }

    // ── LpmTrie ──────────────────────────────────────────────────────────────

    #[test]
    fn bpfmap_lpm_returns_longest_matching_prefix() {
        let (_c, _t) =
            cilium_test_ctx!("bpf/lib/maps.h", "BPF_MAP_TYPE_LPM_TRIE", "tenant-bpf-lpm");
        let mut t: LpmTrie<u32> = LpmTrie::new();
        t.insert("10.0.0.0/8", 1).unwrap();
        t.insert("10.10.0.0/16", 2).unwrap();
        t.insert("10.10.5.0/24", 3).unwrap();
        assert_eq!(t.lookup(ip(10, 10, 5, 7)), Some(3));
        assert_eq!(t.lookup(ip(10, 10, 9, 1)), Some(2));
        assert_eq!(t.lookup(ip(10, 99, 0, 1)), Some(1));
        assert_eq!(t.lookup(ip(11, 0, 0, 1)), None);
    }

    #[test]
    fn bpfmap_lpm_invalid_cidr_rejected_on_insert() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_LPM_TRIE.Validate",
            "tenant-bpf-lpm-bad"
        );
        let mut t: LpmTrie<u32> = LpmTrie::new();
        let err = t.insert("not-a-cidr", 1).unwrap_err();
        assert_eq!(err, MapError::BadCidr("not-a-cidr".into()));
    }

    #[test]
    fn bpfmap_lpm_v6_prefix_match() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_LPM_TRIE.IPv6",
            "tenant-bpf-lpm-v6"
        );
        let mut t: LpmTrie<u32> = LpmTrie::new();
        t.insert("2001:db8::/32", 100).unwrap();
        t.insert("2001:db8:abcd::/48", 200).unwrap();
        let ip6: IpAddr = "2001:db8:abcd::1".parse().unwrap();
        assert_eq!(t.lookup(ip6), Some(200));
    }

    #[test]
    fn bpfmap_lpm_remove_drops_entry() {
        let (_c, _t) = cilium_test_ctx!(
            "bpf/lib/maps.h",
            "BPF_MAP_TYPE_LPM_TRIE.Remove",
            "tenant-bpf-lpm-rm"
        );
        let mut t: LpmTrie<u32> = LpmTrie::new();
        t.insert("10.0.0.0/8", 1).unwrap();
        assert!(t.remove("10.0.0.0/8"));
        assert_eq!(t.lookup(ip(10, 1, 1, 1)), None);
    }

    // ── MapId ────────────────────────────────────────────────────────────────

    #[test]
    fn bpfmap_id_known_paths() {
        let (_c, _t) = cilium_test_ctx!("pkg/datapath/maps/maps.go", "MapID", "tenant-bpf-id");
        assert_eq!(MapId::Endpoints.upstream_path(), "bpf/cilium_endpoints.h");
        assert_eq!(MapId::Ipcache.upstream_path(), "bpf/cilium_ipcache.h");
        assert_eq!(MapId::Policy.upstream_path(), "bpf/cilium_policy.h");
        assert_eq!(MapId::CtTcp.upstream_path(), "bpf/cilium_ct_tcp4.h");
        assert_eq!(MapId::CtAny.upstream_path(), "bpf/cilium_ct_any4.h");
        assert_eq!(MapId::Nat.upstream_path(), "bpf/cilium_snat_v4_external.h");
        assert_eq!(MapId::Lb.upstream_path(), "bpf/cilium_lb4_services.h");
        assert_eq!(
            MapId::LbBackends.upstream_path(),
            "bpf/cilium_lb4_backends.h"
        );
        assert_eq!(MapId::Lxc.upstream_path(), "bpf/cilium_lxc.h");
    }

    #[test]
    fn bpfmap_id_round_trips_serde() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/datapath/maps/maps.go",
            "MapID.Serde",
            "tenant-bpf-id-serde"
        );
        for id in [
            MapId::Endpoints,
            MapId::Ipcache,
            MapId::Policy,
            MapId::CtTcp,
            MapId::Lb,
        ] {
            let s = serde_json::to_string(&id).unwrap();
            let back: MapId = serde_json::from_str(&s).unwrap();
            assert_eq!(back, id);
        }
    }
}
