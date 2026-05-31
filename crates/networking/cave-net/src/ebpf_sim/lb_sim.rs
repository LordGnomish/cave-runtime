// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Userspace simulation of Cilium's IPv4 load-balancer datapath.
//!
//! Cite: cilium/bpf/lib/lb.h (v1.19.3) — `lb4_lookup_service`,
//!       `lb4_lookup_backend_slot`, `lb4_lookup_backend`,
//!       `lb4_select_backend_id_random`, `lb4_select_backend_id_maglev`,
//!       `lb4_xlate` (forward DNAT), `__lb4_rev_nat` (reply restore);
//!       cilium/bpf/lib/hash.h `__hash_from_tuple_v4` (jhash_3words);
//!       cilium/bpf/lib/jhash.h `jhash_3words` (Bob Jenkins lookup3).
//!
//! This is a **userspace datapath approximation**, not a stub. It
//! reproduces the observable forward/reverse service translation:
//!
//!   * **Service lookup.** A service frontend (`backend_slot == 0`)
//!     stores the backend count + reverse-NAT index + LB algorithm.
//!     Slots `1..=count` map to backend IDs; backend IDs resolve to
//!     `(address, port)`. Mirrors the `cilium_lb4_services_v2` +
//!     `cilium_lb4_backends_v3` map pair.
//!   * **Backend selection.** *Random* picks slot
//!     `(prandom % count) + 1` (slot 0 is reserved for the frontend).
//!     *Maglev* hashes the tuple — `jhash_3words(saddr,
//!     (dport<<16)|sport, nexthdr, HASH_INIT4_SEED) % LB_MAGLEV_LUT_SIZE`
//!     — into a precomputed lookup table, giving consistent hashing.
//!     The daddr is excluded from the hash so the same client lands on
//!     the same backend across different service VIPs (upstream
//!     `hash.h` comment).
//!   * **Forward DNAT.** Rewrites the destination to the selected
//!     `backend.address:port` (`lb4_xlate`).
//!   * **Reverse NAT.** A reply from the backend has its source
//!     restored to the service VIP + port via the reverse-NAT index
//!     (`__lb4_rev_nat`).
//!
//!   * **Session affinity.** When a service carries a non-zero
//!     `affinity_timeout`, the first packet of a connection selects a
//!     backend and records `(client_ip, rev_nat_id) -> (backend_id,
//!     last_used)` in `cilium_lb4_affinity`. Packets within the window
//!     reuse that backend (the window slides on each hit); the entry
//!     drops on timeout or when the backend leaves the service
//!     (`cilium_lb_affinity_match` miss). Mirrors
//!     `__lb4_affinity_backend_id` / `__lb4_update_affinity`.
//!
//! Out of scope (kernel BPF harness owns these): packet-buffer writes,
//! L3/L4 checksum recomputation, and the Linux netfilter loopback-SNAT
//! corner case.

use crate::ebpf_sim::map::{Map, UpdateFlag};
use serde::{Deserialize, Serialize};

/// `LB_MAGLEV_LUT_SIZE` — `bpf/node_config.h` (prime, for even spread).
pub const LB_MAGLEV_LUT_SIZE: u32 = 32749;
/// `HASH_INIT4_SEED` — `bpf/node_config.h`.
pub const HASH_INIT4_SEED: u32 = 0xcafe;
/// `JHASH_INITVAL` — `bpf/lib/jhash.h`.
const JHASH_INITVAL: u32 = 0xdead_beef;
/// `NSEC_PER_SEC` — `bpf_sec_to_mono()` scales the service's
/// second-granularity `affinity_timeout` to the monotonic clock.
const NSEC_PER_SEC: u64 = 1_000_000_000;

/// Backend-selection algorithm. Upstream encodes this in the upper
/// 8 bits of the master entry's `affinity_timeout` union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LbAlgo {
    /// `LB_SELECTION_RANDOM` (1).
    Random,
    /// `LB_SELECTION_MAGLEV` (2).
    Maglev,
}

/// `struct lb4_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LbKey {
    pub address: u32,
    pub dport: u16,
    pub backend_slot: u16,
    pub proto: u8,
}

/// `struct lb4_backend` reduced to the DNAT it encodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbBackend {
    pub address: u32,
    pub port: u16,
}

/// `struct lb4_reverse_nat` — the service VIP + port to restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevNatEntry {
    pub address: u32,
    pub port: u16,
}

/// 5-tuple the LB path keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LbTuple {
    pub saddr: u32,
    pub daddr: u32,
    pub sport: u16,
    pub dport: u16,
    pub nexthdr: u8,
}

/// `struct lb4_service` for the master (frontend) entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbServiceMaster {
    pub backend_count: u16,
    pub rev_nat_index: u16,
    pub algorithm: LbAlgo,
    /// `lb4_service.affinity_timeout` (seconds). `0` ⇒ session
    /// affinity disabled for this service.
    pub affinity_timeout: u32,
}

/// `struct lb4_affinity_key` — what a sticky session keys on. The
/// `client_id` is the source IP (`client_ip`) for the by-addr path
/// (the by-cookie path keys on the socket netns cookie instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LbAffinityKey {
    pub client_id: u32,
    pub rev_nat_id: u16,
    /// `netns_cookie` bit — false for the by-addr (external client)
    /// path, true for the host-local socket path.
    pub netns_cookie: bool,
}

/// `struct lb_affinity_val` — the pinned backend and its last touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbAffinityVal {
    pub last_used: u64,
    pub backend_id: u32,
}

/// Result of the forward DNAT path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LbXlate {
    pub backend_id: u32,
    pub new_daddr: u32,
    pub new_dport: u16,
    pub rev_nat_index: u16,
}

/// Result of the reverse-NAT path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevNatResult {
    pub new_saddr: u32,
    pub new_sport: u16,
}

#[inline]
fn rol32(word: u32, shift: u32) -> u32 {
    word.rotate_left(shift)
}

/// `jhash_3words` — Bob Jenkins lookup3 final mix over three words.
/// Faithful port of `bpf/lib/jhash.h`.
pub fn jhash_3words(a: u32, b: u32, c: u32, initval: u32) -> u32 {
    let iv = initval
        .wrapping_add(JHASH_INITVAL)
        .wrapping_add(3 << 2);
    let (mut a, mut b, mut c) = (a.wrapping_add(iv), b.wrapping_add(iv), c.wrapping_add(iv));
    // __jhash_final(a, b, c)
    c ^= b;
    c = c.wrapping_sub(rol32(b, 14));
    a ^= c;
    a = a.wrapping_sub(rol32(c, 11));
    b ^= a;
    b = b.wrapping_sub(rol32(a, 25));
    c ^= b;
    c = c.wrapping_sub(rol32(b, 16));
    a ^= c;
    a = a.wrapping_sub(rol32(c, 4));
    b ^= a;
    b = b.wrapping_sub(rol32(a, 14));
    c ^= b;
    c = c.wrapping_sub(rol32(b, 24));
    c
}

/// `__hash_from_tuple_v4`, then `% LB_MAGLEV_LUT_SIZE`. The daddr is
/// excluded so the same client maps to the same slot across VIPs.
pub fn maglev_index(tuple: &LbTuple) -> u32 {
    let b = ((tuple.dport as u32) << 16) | (tuple.sport as u32);
    jhash_3words(tuple.saddr, b, tuple.nexthdr as u32, HASH_INIT4_SEED) % LB_MAGLEV_LUT_SIZE
}

/// `lb4_select_backend_id_random`: slot = (prandom % count) + 1.
/// Slot 0 is the frontend, so backends live at slots `1..=count`.
pub fn select_backend_id_random(count: u16, prandom: u32) -> u16 {
    ((prandom % count as u32) as u16) + 1
}

/// The LB map set: services (frontend + slots), backends, reverse-NAT
/// entries, and per-service maglev lookup tables.
#[derive(Debug)]
pub struct LbMaps {
    services: Map<LbKey, LbServiceMaster>,
    slots: Map<LbKey, u32>,
    backends: Map<u32, LbBackend>,
    rev_nat: Map<u16, RevNatEntry>,
    /// rev_nat_index -> maglev LUT (each entry a backend_id).
    maglev: std::collections::BTreeMap<u16, Vec<u32>>,
    /// `cilium_lb4_affinity` — sticky session entries.
    affinity: Map<LbAffinityKey, LbAffinityVal>,
    /// `cilium_lb_affinity_match` — the set of `(rev_nat_id,
    /// backend_id)` pairs that are still valid members of a service.
    /// A miss means the backend was removed, so the sticky entry is
    /// stale and must be dropped.
    affinity_match: std::collections::BTreeSet<(u16, u32)>,
}

impl LbMaps {
    pub fn new() -> Self {
        Self {
            services: Map::new_hash(),
            slots: Map::new_hash(),
            backends: Map::new_hash(),
            rev_nat: Map::new_hash(),
            maglev: std::collections::BTreeMap::new(),
            affinity: Map::new_hash(),
            affinity_match: std::collections::BTreeSet::new(),
        }
    }

    fn frontend_key(addr: u32, dport: u16, proto: u8) -> LbKey {
        LbKey { address: addr, dport, backend_slot: 0, proto }
    }

    /// Populate a service with session affinity disabled
    /// (`affinity_timeout == 0`). Thin wrapper over
    /// [`add_service_with_affinity`](Self::add_service_with_affinity).
    pub fn add_service(
        &mut self,
        addr: u32,
        dport: u16,
        proto: u8,
        rev_nat_index: u16,
        algorithm: LbAlgo,
        backends: &[(u32, LbBackend)],
    ) {
        self.add_service_with_affinity(addr, dport, proto, rev_nat_index, algorithm, backends, 0);
    }

    /// Populate a service: frontend entry, per-slot backend mapping,
    /// backend table, reverse-NAT entry, the `affinity_match` set (one
    /// entry per `(rev_nat_index, backend_id)`), and (for maglev) a LUT
    /// that round-robins the configured backends across the table.
    /// `affinity_timeout` is the sticky window in seconds (`0` ⇒ off).
    pub fn add_service_with_affinity(
        &mut self,
        addr: u32,
        dport: u16,
        proto: u8,
        rev_nat_index: u16,
        algorithm: LbAlgo,
        backends: &[(u32, LbBackend)],
        affinity_timeout: u32,
    ) {
        let count = backends.len() as u16;
        self.services
            .update(
                Self::frontend_key(addr, dport, proto),
                LbServiceMaster { backend_count: count, rev_nat_index, algorithm, affinity_timeout },
                UpdateFlag::Any,
            )
            .expect("frontend insert");
        for (slot, (backend_id, backend)) in backends.iter().enumerate() {
            let key = LbKey { address: addr, dport, backend_slot: (slot as u16) + 1, proto };
            self.slots.update(key, *backend_id, UpdateFlag::Any).expect("slot insert");
            self.backends.update(*backend_id, *backend, UpdateFlag::Any).expect("backend insert");
            // Control plane records every active backend in the
            // affinity-match map so the datapath can detect removals.
            self.affinity_match.insert((rev_nat_index, *backend_id));
        }
        self.rev_nat
            .update(rev_nat_index, RevNatEntry { address: addr, port: dport }, UpdateFlag::Any)
            .expect("rev_nat insert");
        if algorithm == LbAlgo::Maglev {
            // Build a LUT: backend_ids round-robined across all slots.
            // The agent computes a true Maglev permutation; for the
            // datapath sim what matters is a deterministic table that
            // every backend appears in — consistency is the property
            // under test.
            let ids: Vec<u32> = backends.iter().map(|(id, _)| *id).collect();
            let lut: Vec<u32> = (0..LB_MAGLEV_LUT_SIZE)
                .map(|i| ids[(i as usize) % ids.len()])
                .collect();
            self.maglev.insert(rev_nat_index, lut);
        }
    }

    /// `lb4_lookup_service` — the frontend (slot 0) entry.
    pub fn lb4_lookup_service(&mut self, addr: u32, dport: u16, proto: u8) -> Option<LbServiceMaster> {
        self.services.lookup(&Self::frontend_key(addr, dport, proto))
    }

    /// `lb4_lookup_backend_slot` -> backend_id at `slot` (1-based).
    pub fn lb4_lookup_backend_slot(&mut self, addr: u32, dport: u16, proto: u8, slot: u16) -> Option<u32> {
        self.slots.lookup(&LbKey { address: addr, dport, backend_slot: slot, proto })
    }

    /// `lb4_lookup_backend` -> backend addr/port for an id.
    pub fn lb4_lookup_backend(&mut self, backend_id: u32) -> Option<LbBackend> {
        self.backends.lookup(&backend_id)
    }

    fn xlate(&mut self, addr: u32, dport: u16, proto: u8, slot: u16, rev_nat_index: u16) -> Option<LbXlate> {
        let backend_id = self.lb4_lookup_backend_slot(addr, dport, proto, slot)?;
        let backend = self.lb4_lookup_backend(backend_id)?;
        Some(LbXlate {
            backend_id,
            new_daddr: backend.address,
            new_dport: backend.port,
            rev_nat_index,
        })
    }

    /// Full forward path with the random algorithm; `prandom` is the
    /// `bpf_get_prandom_u32()` value (caller pins it for determinism).
    pub fn lb4_local_random(&mut self, tuple: &LbTuple, prandom: u32) -> Option<LbXlate> {
        let proto = tuple.nexthdr;
        let svc = self.lb4_lookup_service(tuple.daddr, tuple.dport, proto)?;
        if svc.backend_count == 0 {
            return None;
        }
        let slot = select_backend_id_random(svc.backend_count, prandom);
        self.xlate(tuple.daddr, tuple.dport, proto, slot, svc.rev_nat_index)
    }

    /// Full forward path with the maglev algorithm — consistent
    /// hashing on the (daddr-excluded) tuple.
    pub fn lb4_local_maglev(&mut self, tuple: &LbTuple) -> Option<LbXlate> {
        let proto = tuple.nexthdr;
        let svc = self.lb4_lookup_service(tuple.daddr, tuple.dport, proto)?;
        let lut = self.maglev.get(&svc.rev_nat_index)?;
        if lut.is_empty() {
            return None;
        }
        let backend_id = lut[maglev_index(tuple) as usize];
        let backend = self.lb4_lookup_backend(backend_id)?;
        Some(LbXlate {
            backend_id,
            new_daddr: backend.address,
            new_dport: backend.port,
            rev_nat_index: svc.rev_nat_index,
        })
    }

    /// `__lb4_affinity_backend_id` — look up the sticky backend for a
    /// client. Returns `0` (no backend) on any of: no entry, the entry
    /// has aged past `affinity_timeout_sec`, or the recorded backend is
    /// no longer a member of the service (`affinity_match` miss). The
    /// two failure cases also evict the stale entry, mirroring the
    /// `map_delete_elem` calls in `bpf/lib/lb.h`. On a hit the entry's
    /// `last_used` is refreshed to `now_ns` and the backend returned.
    pub fn lb_affinity_backend_id(
        &mut self,
        rev_nat_id: u16,
        affinity_timeout_sec: u32,
        client_id: u32,
        now_ns: u64,
    ) -> u32 {
        let key = LbAffinityKey { client_id, rev_nat_id, netns_cookie: false };
        let val = match self.affinity.lookup(&key) {
            Some(v) => v,
            None => return 0,
        };
        // bpf_sec_to_mono(affinity_timeout): seconds -> monotonic ns.
        let timeout_ns = (affinity_timeout_sec as u64).saturating_mul(NSEC_PER_SEC);
        // Sticky window is [last_used, last_used + timeout); the upper
        // bound is excluded (`<=` ⇒ expired).
        if val.last_used.saturating_add(timeout_ns) <= now_ns {
            let _ = self.affinity.delete(&key);
            return 0;
        }
        // Backend must still belong to the service.
        if !self.affinity_match.contains(&(rev_nat_id, val.backend_id)) {
            let _ = self.affinity.delete(&key);
            return 0;
        }
        let refreshed = LbAffinityVal { last_used: now_ns, backend_id: val.backend_id };
        let _ = self.affinity.update(key, refreshed, UpdateFlag::Any);
        val.backend_id
    }

    /// `__lb4_update_affinity` — pin `client_id` to `backend_id` for
    /// this service, stamping `last_used = now_ns`. Called on the
    /// `CT_NEW` path after a fresh backend selection.
    pub fn lb_update_affinity(&mut self, rev_nat_id: u16, client_id: u32, backend_id: u32, now_ns: u64) {
        let key = LbAffinityKey { client_id, rev_nat_id, netns_cookie: false };
        let val = LbAffinityVal { last_used: now_ns, backend_id };
        let _ = self.affinity.update(key, val, UpdateFlag::Any);
    }

    /// Remove a `(rev_nat_id, backend_id)` pair from the affinity-match
    /// map — the control-plane action when a backend leaves a service.
    /// Sticky entries still pointing at it become stale on next lookup.
    pub fn remove_affinity_match(&mut self, rev_nat_id: u16, backend_id: u32) {
        self.affinity_match.remove(&(rev_nat_id, backend_id));
    }

    /// Forward path honoring session affinity (the `lb4_local`
    /// `CT_NEW`/`CT_ESTABLISHED` split for an affinity-enabled
    /// service). When the service has a non-zero `affinity_timeout`
    /// and a live sticky entry exists, that backend is reused
    /// regardless of `prandom`; otherwise a backend is selected at
    /// random and recorded. `now_ns` pins the monotonic clock the way
    /// `bpf_mono_now()` reads it. Services with `affinity_timeout == 0`
    /// behave exactly like [`lb4_local_random`](Self::lb4_local_random).
    pub fn lb4_local_affinity(&mut self, tuple: &LbTuple, prandom: u32, now_ns: u64) -> Option<LbXlate> {
        let proto = tuple.nexthdr;
        let svc = self.lb4_lookup_service(tuple.daddr, tuple.dport, proto)?;
        if svc.backend_count == 0 {
            return None;
        }
        let rev = svc.rev_nat_index;
        if svc.affinity_timeout > 0 {
            let bid = self.lb_affinity_backend_id(rev, svc.affinity_timeout, tuple.saddr, now_ns);
            if bid != 0 {
                let backend = self.lb4_lookup_backend(bid)?;
                return Some(LbXlate {
                    backend_id: bid,
                    new_daddr: backend.address,
                    new_dport: backend.port,
                    rev_nat_index: rev,
                });
            }
        }
        // CT_NEW: select a backend, then record the affinity.
        let slot = select_backend_id_random(svc.backend_count, prandom);
        let xlate = self.xlate(tuple.daddr, tuple.dport, proto, slot, rev)?;
        if svc.affinity_timeout > 0 {
            self.lb_update_affinity(rev, tuple.saddr, xlate.backend_id, now_ns);
        }
        Some(xlate)
    }

    /// `lb4_rev_nat` — restore the service VIP + port for a reply.
    /// `_reply` carries the reply 5-tuple (source is the backend); the
    /// reverse-NAT index identifies which service to restore.
    pub fn lb4_rev_nat(&mut self, rev_nat_index: u16, _reply: &LbTuple) -> Option<RevNatResult> {
        let nat = self.rev_nat.lookup(&rev_nat_index)?;
        Some(RevNatResult { new_saddr: nat.address, new_sport: nat.port })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ebpf_sim::program::L4Proto;

    #[test]
    fn random_slot_never_zero_and_in_range() {
        for p in 0..100u32 {
            let slot = select_backend_id_random(4, p);
            assert!((1..=4).contains(&slot));
        }
    }

    #[test]
    fn jhash_3words_pure_and_seed_sensitive() {
        assert_eq!(jhash_3words(7, 8, 9, 0xcafe), jhash_3words(7, 8, 9, 0xcafe));
        assert_ne!(jhash_3words(7, 8, 9, 0), jhash_3words(7, 8, 9, 1));
    }

    #[test]
    fn maglev_index_in_range_and_daddr_independent() {
        let a = LbTuple { saddr: 0x0A000009, daddr: 1, sport: 33333, dport: 80, nexthdr: L4Proto::Tcp.proto_num() };
        let b = LbTuple { daddr: 999, ..a };
        assert_eq!(maglev_index(&a), maglev_index(&b));
        assert!(maglev_index(&a) < LB_MAGLEV_LUT_SIZE);
    }

    #[test]
    fn forward_random_resolves_backend() {
        let mut maps = LbMaps::new();
        maps.add_service(
            0xAC140001,
            80,
            L4Proto::Tcp.proto_num(),
            5,
            LbAlgo::Random,
            &[(1, LbBackend { address: 0x0A010001, port: 8080 })],
        );
        let t = LbTuple { saddr: 0x0A000001, daddr: 0xAC140001, sport: 5000, dport: 80, nexthdr: L4Proto::Tcp.proto_num() };
        let x = maps.lb4_local_random(&t, 0).unwrap();
        assert_eq!(x.backend_id, 1);
        assert_eq!(x.new_dport, 8080);
        assert_eq!(x.rev_nat_index, 5);
    }
}
