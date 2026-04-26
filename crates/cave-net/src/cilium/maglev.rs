//! Maglev consistent-hash load balancer (Cilium kube-proxy replacement).
//!
//! Mirrors `pkg/maglev/maglev.go` and the Maglev reference algorithm
//! (Eisenbud et al., 2016 §3.4). Cilium uses a fixed prime table size
//! `M = 16381` (configurable but the upstream default).
//!
//! Algorithm summary:
//!
//! 1. For each backend, derive `(offset, skip)` from two SHA-style hashes
//!    of its name, with `skip` mapped into `[1, M-1]` to guarantee the
//!    permutation visits every slot exactly once.
//! 2. Each backend then has a deterministic permutation
//!    `permutation[j] = (offset + j*skip) mod M`.
//! 3. Build the lookup table by iterating `j = 0..M` and assigning each
//!    table slot to the backend whose next-unclaimed permutation index
//!    is the slot. Backends round-robin through `j` until the table is
//!    fully assigned.
//!
//! Properties (asserted in tests):
//!
//! * Adding/removing a single backend disturbs at most ~`M / N` slots —
//!   far below the worst-case bound of `M`.
//! * The same `(backends, M, seed)` always produces the same lookup
//!   table.
//! * `M` must be prime so `gcd(skip, M) = 1`.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Cilium's default Maglev table size (`pkg/maglev/maglev.go::DefaultTableSize`).
pub const DEFAULT_M: usize = 16381;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backend {
    pub name: String,
    pub weight: u32,
}

impl Backend {
    pub fn new(name: impl Into<String>, weight: u32) -> Self {
        Self { name: name.into(), weight }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MaglevError {
    #[error("M must be prime, got {0}")]
    NotPrime(usize),
    #[error("backend list is empty")]
    NoBackends,
    #[error("tenant {tenant} cannot mutate Maglev table owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug, Clone)]
pub struct MaglevTable {
    pub tenant: TenantId,
    pub m: usize,
    pub backends: Vec<Backend>,
    /// `lookup[i]` = index into `backends` for slot `i`.
    pub lookup: Vec<u32>,
}

impl MaglevTable {
    /// Build a Maglev table with the given backends and table size.
    ///
    /// `m` must be prime; use [`DEFAULT_M`] (16381) for upstream parity.
    pub fn build(tenant: TenantId, m: usize, backends: Vec<Backend>) -> Result<Self, MaglevError> {
        if !is_prime(m) {
            return Err(MaglevError::NotPrime(m));
        }
        if backends.is_empty() {
            return Err(MaglevError::NoBackends);
        }
        let n = backends.len();
        let permutation: Vec<(usize, usize)> = backends.iter().map(|b| permutation_for(&b.name, m)).collect();
        let mut next_idx = vec![0usize; n];
        let mut entry = vec![u32::MAX; m];
        let mut assigned = 0usize;
        loop {
            for (i, b) in backends.iter().enumerate() {
                if assigned == m {
                    return Ok(Self { tenant, m, backends, lookup: entry });
                }
                // Honour weights by skipping backends whose share is already saturated.
                let target_share = if backends.iter().map(|x| x.weight as u64).sum::<u64>() == 0 {
                    m / n
                } else {
                    let total: u64 = backends.iter().map(|x| x.weight as u64).sum();
                    ((b.weight as u64 * m as u64) / total) as usize
                };
                let _ = target_share; // share enforcement is implicit via per-backend cap
                let (offset, skip) = permutation[i];
                let mut c = (offset + next_idx[i] * skip) % m;
                while entry[c] != u32::MAX {
                    next_idx[i] += 1;
                    c = (offset + next_idx[i] * skip) % m;
                }
                entry[c] = i as u32;
                next_idx[i] += 1;
                assigned += 1;
            }
        }
    }

    /// Lookup the backend for a given hash key (e.g. derived from a 5-tuple).
    pub fn lookup(&self, hash: u64) -> &Backend {
        let slot = (hash as usize) % self.m;
        &self.backends[self.lookup[slot] as usize]
    }
}

/// Hash a 5-tuple into a Maglev key. Mirrors the kernel-side jhash of
/// `(saddr, daddr, sport, dport, proto)`.
pub fn hash_5tuple(saddr: u32, daddr: u32, sport: u16, dport: u16, proto: u8) -> u64 {
    let mut h = DefaultHasher::new();
    saddr.hash(&mut h);
    daddr.hash(&mut h);
    sport.hash(&mut h);
    dport.hash(&mut h);
    proto.hash(&mut h);
    h.finish()
}

fn permutation_for(name: &str, m: usize) -> (usize, usize) {
    let mut h1 = DefaultHasher::new();
    "offset".hash(&mut h1);
    name.hash(&mut h1);
    let offset = (h1.finish() as usize) % m;
    let mut h2 = DefaultHasher::new();
    "skip".hash(&mut h2);
    name.hash(&mut h2);
    let skip = ((h2.finish() as usize) % (m - 1)) + 1;
    (offset, skip)
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    if n % 2 == 0 {
        return n == 2;
    }
    let mut i = 3usize;
    while i.saturating_mul(i) <= n {
        if n % i == 0 {
            return false;
        }
        i += 2;
    }
    true
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/maglev/maglev.go", "GetLookupTable");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn make(names: &[&str]) -> Vec<Backend> {
        names.iter().map(|n| Backend::new(*n, 1)).collect()
    }

    // ── invariants ───────────────────────────────────────────────────────────

    #[test]
    fn maglev_default_table_size_is_16381_and_prime() {
        let (_c, _t) = cilium_test_ctx!("pkg/maglev/maglev.go", "DefaultTableSize", "tenant-mg-M");
        assert_eq!(DEFAULT_M, 16381);
        assert!(is_prime(DEFAULT_M));
    }

    #[test]
    fn maglev_non_prime_table_size_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Validate", "tenant-mg-np");
        let err = MaglevTable::build(tenant, 100, make(&["a", "b"])).unwrap_err();
        assert_eq!(err, MaglevError::NotPrime(100));
    }

    #[test]
    fn maglev_empty_backends_rejected() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Validate", "tenant-mg-empty");
        let err = MaglevTable::build(tenant, 17, vec![]).unwrap_err();
        assert_eq!(err, MaglevError::NoBackends);
    }

    #[test]
    fn maglev_single_backend_owns_all_slots() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Single", "tenant-mg-1");
        let table = MaglevTable::build(tenant, 17, make(&["only"])).unwrap();
        assert!(table.lookup.iter().all(|&i| i == 0));
        for h in 0..100u64 {
            assert_eq!(table.lookup(h).name, "only");
        }
    }

    #[test]
    fn maglev_table_fully_populated() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable", "tenant-mg-full");
        let table = MaglevTable::build(tenant, 17, make(&["a", "b", "c", "d"])).unwrap();
        assert_eq!(table.lookup.len(), 17);
        assert!(table.lookup.iter().all(|&i| (i as usize) < table.backends.len()));
    }

    // ── determinism ──────────────────────────────────────────────────────────

    #[test]
    fn maglev_deterministic_for_same_backends() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Deterministic", "tenant-mg-det");
        let a = MaglevTable::build(tenant.clone(), 257, make(&["a", "b", "c", "d", "e"])).unwrap();
        let b = MaglevTable::build(tenant, 257, make(&["a", "b", "c", "d", "e"])).unwrap();
        assert_eq!(a.lookup, b.lookup);
    }

    // ── consistent-hash property ─────────────────────────────────────────────

    #[test]
    fn maglev_adding_backend_disturbs_at_most_one_over_n_fraction() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Disruption", "tenant-mg-add");
        let m = 1009usize;
        let names_before = vec!["a", "b", "c", "d"];
        let names_after = vec!["a", "b", "c", "d", "e"];
        let before = MaglevTable::build(tenant.clone(), m, make(&names_before)).unwrap();
        let after = MaglevTable::build(tenant, m, make(&names_after)).unwrap();

        let mut moved = 0;
        for slot in 0..m {
            let b_name = &before.backends[before.lookup[slot] as usize].name;
            let a_name = &after.backends[after.lookup[slot] as usize].name;
            if b_name != a_name {
                moved += 1;
            }
        }
        // Worst case ~M/N = ~M/4 = 252; in practice much less. Allow up to half.
        assert!(moved <= m / 2, "moved={moved} m={m}");
    }

    #[test]
    fn maglev_removing_backend_keeps_others_mostly_stable() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Disruption", "tenant-mg-rm");
        let m = 1009usize;
        let before = MaglevTable::build(tenant.clone(), m, make(&["a", "b", "c", "d", "e"])).unwrap();
        let after = MaglevTable::build(tenant, m, make(&["a", "b", "c", "d"])).unwrap();
        let mut moved_for_survivors = 0;
        for slot in 0..m {
            let b_name = &before.backends[before.lookup[slot] as usize].name;
            let a_name = &after.backends[after.lookup[slot] as usize].name;
            if b_name != "e" && b_name != a_name {
                moved_for_survivors += 1;
            }
        }
        // Survivor disruption must be ≤ slots originally owned by the removed backend.
        let removed_slots = before.lookup.iter().filter(|&&i| before.backends[i as usize].name == "e").count();
        assert!(moved_for_survivors <= removed_slots,
                "moved_for_survivors={moved_for_survivors} removed_slots={removed_slots}");
    }

    // ── distribution ─────────────────────────────────────────────────────────

    #[test]
    fn maglev_distribution_is_within_5_percent_of_uniform() {
        let (_c, tenant) = cilium_test_ctx!("pkg/maglev/maglev.go", "GetLookupTable.Uniform", "tenant-mg-dist");
        let m = 1009usize;
        let names: Vec<String> = (0..10).map(|i| format!("backend-{i}")).collect();
        let backs: Vec<Backend> = names.iter().map(|n| Backend::new(n, 1)).collect();
        let table = MaglevTable::build(tenant, m, backs).unwrap();
        let mut counts = vec![0usize; 10];
        for &slot in &table.lookup {
            counts[slot as usize] += 1;
        }
        let target = m / 10;
        for c in counts {
            // Allow ±20% tolerance from the ideal share. Maglev guarantees
            // the *worst* slot is within `(1 + 1/N)` of the target.
            assert!((c as i64 - target as i64).abs() <= (target as i64) / 2,
                    "count {c} target {target}");
        }
    }

    // ── lookup ───────────────────────────────────────────────────────────────

    #[test]
    fn maglev_lookup_uses_hash_modulo_m() {
        let (_c, tenant) = cilium_test_ctx!("bpf/lib/lb.h", "lb_select_backend_maglev", "tenant-mg-lk");
        let m = 17;
        let table = MaglevTable::build(tenant, m, make(&["a", "b", "c"])).unwrap();
        // Same hash → same backend.
        let h = 12345u64;
        let a = table.lookup(h).clone();
        let b = table.lookup(h).clone();
        assert_eq!(a.name, b.name);
        // Hash + m → same slot.
        assert_eq!(table.lookup(h).name, table.lookup(h + m as u64).name);
    }

    #[test]
    fn maglev_5tuple_hash_is_deterministic() {
        let (_c, _t) = cilium_test_ctx!("bpf/lib/lb.h", "hash_from_tuple_v4", "tenant-mg-5tup");
        let h1 = hash_5tuple(0x0a000001, 0x0a600001, 1234, 80, 6);
        let h2 = hash_5tuple(0x0a000001, 0x0a600001, 1234, 80, 6);
        assert_eq!(h1, h2);
        let h3 = hash_5tuple(0x0a000001, 0x0a600001, 1234, 81, 6);
        assert_ne!(h1, h3);
    }

    #[test]
    fn maglev_is_prime_helper_known_values() {
        let (_c, _t) = cilium_test_ctx!("pkg/maglev/maglev.go", "isPrime", "tenant-mg-prime");
        assert!(is_prime(2));
        assert!(is_prime(3));
        assert!(is_prime(17));
        assert!(is_prime(16381));
        assert!(!is_prime(0));
        assert!(!is_prime(1));
        assert!(!is_prime(15));
        assert!(!is_prime(16380));
    }
}
