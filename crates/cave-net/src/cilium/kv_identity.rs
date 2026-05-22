// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! KVStore identity allocator — cluster-wide identity allocation backend.
//!
//! Mirrors `pkg/identity/cache/global.go::CRDBackedAllocator` plus the
//! KVStore-backed allocator in `pkg/identity/cache/kvstore_allocator.go`.
//! Where [`super::identity::LocalIdentityCache`] is per-agent, this
//! allocator is **cluster-wide**: one identity per label-set across the
//! whole cluster, coordinated through the KVStore (etcd) so all agents
//! see the same numeric IDs.
//!
//! Semantics (faithful to upstream):
//!
//! * The same label set seen on two different nodes resolves to the
//!   same identity (mirrors upstream's `master key` lock-then-allocate).
//! * Per-allocation refcounts: when the last referrer releases, the
//!   master key is GC'd after a grace period.
//! * Reserved identities (1..=255) are never allocated by this backend
//!   — they're hard-coded constants used directly.
//! * Allocation is monotonic above [`MIN_GLOBAL_IDENTITY`] (= 1<<24)
//!   so it doesn't collide with the local-cache range.

use crate::cilium::identity::{LabelSet, MIN_LOCAL_IDENTITY};
use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lowest identity the KVStore allocator may issue.
/// Sits at 2^24 to avoid collision with reserved (1..256) and local
/// cache (256..2^24) ranges.
pub const MIN_GLOBAL_IDENTITY: u32 = 1 << 24;
pub const MAX_GLOBAL_IDENTITY: u32 = u32::MAX - 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalIdentity {
    pub identity: u32,
    pub labels: LabelSet,
    pub refcount: u32,
    /// Time of the last release that took the refcount to zero.
    /// `None` while still referenced.
    pub released_at_ns: Option<u64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KvIdentityError {
    #[error("label set is empty")]
    EmptyLabels,
    #[error("identity {0} below minimum global range")]
    BelowGlobalRange(u32),
    #[error("identity space exhausted (allocated {0})")]
    Exhausted(u32),
    #[error("identity {0} not found")]
    NotFound(u32),
    #[error("tenant {tenant} cannot mutate global allocator owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

#[derive(Debug)]
pub struct KvIdentityAllocator {
    pub tenant: TenantId,
    pub gc_grace_seconds: u64,
    next_id: u32,
    by_id: HashMap<u32, GlobalIdentity>,
    by_labels: HashMap<LabelSet, u32>,
}

impl KvIdentityAllocator {
    pub fn new(tenant: TenantId, gc_grace_seconds: u64) -> Self {
        Self {
            tenant,
            gc_grace_seconds,
            next_id: MIN_GLOBAL_IDENTITY,
            by_id: HashMap::new(),
            by_labels: HashMap::new(),
        }
    }

    /// Cluster-wide allocate-or-lookup. Same label set always returns
    /// the same identity. Refcount is incremented on every call.
    pub fn allocate(&mut self, labels: &LabelSet, now_ns: u64) -> Result<u32, KvIdentityError> {
        if labels.pairs.is_empty() {
            return Err(KvIdentityError::EmptyLabels);
        }
        if let Some(&existing_id) = self.by_labels.get(labels) {
            if let Some(entry) = self.by_id.get_mut(&existing_id) {
                entry.refcount += 1;
                entry.released_at_ns = None;
            }
            return Ok(existing_id);
        }
        if self.next_id > MAX_GLOBAL_IDENTITY {
            return Err(KvIdentityError::Exhausted(self.next_id - 1));
        }
        let id = self.next_id;
        self.next_id += 1;
        let entry = GlobalIdentity {
            identity: id,
            labels: labels.clone(),
            refcount: 1,
            released_at_ns: None,
        };
        self.by_id.insert(id, entry);
        self.by_labels.insert(labels.clone(), id);
        let _ = now_ns;
        Ok(id)
    }

    pub fn release(&mut self, id: u32, now_ns: u64) -> Result<(), KvIdentityError> {
        if id < MIN_GLOBAL_IDENTITY {
            return Err(KvIdentityError::BelowGlobalRange(id));
        }
        let entry = self
            .by_id
            .get_mut(&id)
            .ok_or(KvIdentityError::NotFound(id))?;
        if entry.refcount > 0 {
            entry.refcount -= 1;
        }
        if entry.refcount == 0 {
            entry.released_at_ns = Some(now_ns);
        }
        Ok(())
    }

    pub fn lookup(&self, id: u32) -> Option<&GlobalIdentity> {
        self.by_id.get(&id)
    }

    pub fn lookup_by_labels(&self, labels: &LabelSet) -> Option<u32> {
        self.by_labels.get(labels).copied()
    }

    pub fn refcount(&self, id: u32) -> u32 {
        self.by_id.get(&id).map(|e| e.refcount).unwrap_or(0)
    }

    pub fn count(&self) -> usize {
        self.by_id.len()
    }

    /// GC entries whose refcount has been zero for at least
    /// `gc_grace_seconds`. Returns count removed.
    pub fn gc(&mut self, now_ns: u64) -> usize {
        let grace_ns = self.gc_grace_seconds * 1_000_000_000;
        let dead: Vec<u32> = self
            .by_id
            .iter()
            .filter(|(_, e)| {
                if e.refcount > 0 {
                    return false;
                }
                match e.released_at_ns {
                    Some(t) => now_ns.saturating_sub(t) >= grace_ns,
                    None => false,
                }
            })
            .map(|(k, _)| *k)
            .collect();
        let n = dead.len();
        for id in dead {
            if let Some(e) = self.by_id.remove(&id) {
                self.by_labels.remove(&e.labels);
            }
        }
        n
    }

    /// Used by ClusterMesh sync — restore a global identity from a peer.
    pub fn restore(&mut self, identity: u32, labels: LabelSet) -> Result<(), KvIdentityError> {
        if identity < MIN_LOCAL_IDENTITY {
            return Err(KvIdentityError::BelowGlobalRange(identity));
        }
        if labels.pairs.is_empty() {
            return Err(KvIdentityError::EmptyLabels);
        }
        let entry = GlobalIdentity {
            identity,
            labels: labels.clone(),
            refcount: 1,
            released_at_ns: None,
        };
        self.by_labels.insert(labels, identity);
        self.by_id.insert(identity, entry);
        if identity >= self.next_id {
            self.next_id = identity + 1;
        }
        Ok(())
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium(
    "pkg/identity/cache/kvstore_allocator.go",
    "KVStoreAllocator",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    fn alloc(tenant: TenantId) -> KvIdentityAllocator {
        KvIdentityAllocator::new(tenant, 60)
    }

    // ── Range / constants ───────────────────────────────────────────────────

    #[test]
    fn min_global_identity_at_2_to_24() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "MinGlobal",
            "tenant-kvi-min"
        );
        assert_eq!(MIN_GLOBAL_IDENTITY, 1 << 24);
    }

    // ── Allocate ────────────────────────────────────────────────────────────

    #[test]
    fn allocate_first_call_starts_at_min_global() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Allocate.First",
            "tenant-kvi-af"
        );
        let mut a = alloc(tenant);
        let id = a.allocate(&ls(&[("app", "web")]), 100).unwrap();
        assert_eq!(id, MIN_GLOBAL_IDENTITY);
    }

    #[test]
    fn allocate_same_labels_returns_same_id_and_bumps_refcount() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Allocate.SameLabels",
            "tenant-kvi-asl"
        );
        let mut a = alloc(tenant);
        let labels = ls(&[("app", "web")]);
        let a1 = a.allocate(&labels, 100).unwrap();
        let a2 = a.allocate(&labels, 100).unwrap();
        assert_eq!(a1, a2);
        assert_eq!(a.refcount(a1), 2);
    }

    #[test]
    fn allocate_distinct_labels_returns_distinct_ids() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Allocate.Distinct",
            "tenant-kvi-ad"
        );
        let mut a = alloc(tenant);
        let id1 = a.allocate(&ls(&[("app", "web")]), 100).unwrap();
        let id2 = a.allocate(&ls(&[("app", "api")]), 100).unwrap();
        assert_ne!(id1, id2);
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn allocate_empty_labels_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Allocate.EmptyLabels",
            "tenant-kvi-ae"
        );
        let mut a = alloc(tenant);
        let err = a.allocate(&LabelSet { pairs: vec![] }, 100).unwrap_err();
        assert_eq!(err, KvIdentityError::EmptyLabels);
    }

    // ── Lookup ──────────────────────────────────────────────────────────────

    #[test]
    fn lookup_by_id_returns_entry() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Lookup.ByID",
            "tenant-kvi-lid"
        );
        let mut a = alloc(tenant);
        let labels = ls(&[("app", "web")]);
        let id = a.allocate(&labels, 100).unwrap();
        let e = a.lookup(id).unwrap();
        assert_eq!(e.labels, labels);
    }

    #[test]
    fn lookup_unknown_id_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Lookup.NotFound",
            "tenant-kvi-lnf"
        );
        let a = alloc(tenant);
        assert!(a.lookup(999).is_none());
    }

    #[test]
    fn lookup_by_labels_returns_id() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Lookup.ByLabels",
            "tenant-kvi-lbl"
        );
        let mut a = alloc(tenant);
        let labels = ls(&[("app", "web")]);
        let id = a.allocate(&labels, 100).unwrap();
        assert_eq!(a.lookup_by_labels(&labels), Some(id));
    }

    #[test]
    fn lookup_by_unknown_labels_returns_none() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Lookup.ByLabels.NotFound",
            "tenant-kvi-lblnf"
        );
        let a = alloc(tenant);
        assert!(a.lookup_by_labels(&ls(&[("nope", "x")])).is_none());
    }

    // ── Release / refcount ──────────────────────────────────────────────────

    #[test]
    fn release_decrements_refcount() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Release",
            "tenant-kvi-rel"
        );
        let mut a = alloc(tenant);
        let labels = ls(&[("app", "web")]);
        let id = a.allocate(&labels, 100).unwrap();
        let _ = a.allocate(&labels, 100).unwrap();
        a.release(id, 200).unwrap();
        assert_eq!(a.refcount(id), 1);
    }

    #[test]
    fn release_below_global_range_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Release.BelowGlobal",
            "tenant-kvi-relbg"
        );
        let mut a = alloc(tenant);
        let err = a.release(1 /* reserved */, 100).unwrap_err();
        assert_eq!(err, KvIdentityError::BelowGlobalRange(1));
    }

    #[test]
    fn release_unknown_returns_not_found() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Release.NotFound",
            "tenant-kvi-relnf"
        );
        let mut a = alloc(tenant);
        let err = a.release(MIN_GLOBAL_IDENTITY + 1, 100).unwrap_err();
        assert_eq!(err, KvIdentityError::NotFound(MIN_GLOBAL_IDENTITY + 1));
    }

    #[test]
    fn release_to_zero_records_release_timestamp() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Release.RecordTimestamp",
            "tenant-kvi-relts"
        );
        let mut a = alloc(tenant);
        let id = a.allocate(&ls(&[("app", "web")]), 100).unwrap();
        a.release(id, 200).unwrap();
        assert_eq!(a.lookup(id).unwrap().released_at_ns, Some(200));
    }

    #[test]
    fn allocate_after_release_re_increments_refcount_and_clears_timestamp() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Reallocate",
            "tenant-kvi-real"
        );
        let mut a = alloc(tenant);
        let labels = ls(&[("app", "web")]);
        let id = a.allocate(&labels, 100).unwrap();
        a.release(id, 200).unwrap();
        let id2 = a.allocate(&labels, 300).unwrap();
        assert_eq!(id2, id);
        assert_eq!(a.lookup(id).unwrap().released_at_ns, None);
        assert_eq!(a.refcount(id), 1);
    }

    // ── GC ──────────────────────────────────────────────────────────────────

    #[test]
    fn gc_removes_released_after_grace() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "GC.Released",
            "tenant-kvi-gc"
        );
        let mut a = alloc(tenant);
        let id = a.allocate(&ls(&[("app", "web")]), 100).unwrap();
        a.release(id, 100).unwrap();
        let n = a.gc(60_000_000_000 + 100);
        assert_eq!(n, 1);
        assert!(a.lookup(id).is_none());
    }

    #[test]
    fn gc_keeps_released_within_grace() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "GC.WithinGrace",
            "tenant-kvi-gcg"
        );
        let mut a = alloc(tenant);
        let id = a.allocate(&ls(&[("app", "web")]), 100).unwrap();
        a.release(id, 100).unwrap();
        let n = a.gc(30_000_000_000);
        assert_eq!(n, 0);
        assert!(a.lookup(id).is_some());
    }

    #[test]
    fn gc_keeps_referenced_entries() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "GC.KeepRef",
            "tenant-kvi-gck"
        );
        let mut a = alloc(tenant);
        let id = a.allocate(&ls(&[("app", "web")]), 100).unwrap();
        let n = a.gc(60_000_000_000 + 100);
        assert_eq!(n, 0);
        assert!(a.lookup(id).is_some());
    }

    // ── Restore from peer (ClusterMesh sync) ────────────────────────────────

    #[test]
    fn restore_inserts_with_provided_identity() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Restore",
            "tenant-kvi-rst"
        );
        let mut a = alloc(tenant);
        let labels = ls(&[("app", "web")]);
        a.restore(MIN_GLOBAL_IDENTITY + 100, labels.clone())
            .unwrap();
        assert_eq!(a.lookup_by_labels(&labels), Some(MIN_GLOBAL_IDENTITY + 100));
    }

    #[test]
    fn restore_empty_labels_rejected() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Restore.Empty",
            "tenant-kvi-rste"
        );
        let mut a = alloc(tenant);
        let err = a
            .restore(MIN_GLOBAL_IDENTITY + 1, LabelSet { pairs: vec![] })
            .unwrap_err();
        assert_eq!(err, KvIdentityError::EmptyLabels);
    }

    #[test]
    fn restore_advances_next_id_to_avoid_collision() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Restore.NextID",
            "tenant-kvi-rstn"
        );
        let mut a = alloc(tenant);
        a.restore(MIN_GLOBAL_IDENTITY + 99, ls(&[("app", "web")]))
            .unwrap();
        let next = a.allocate(&ls(&[("app", "api")]), 100).unwrap();
        assert!(next > MIN_GLOBAL_IDENTITY + 99);
    }

    // ── Count ───────────────────────────────────────────────────────────────

    #[test]
    fn count_tracks_alloc_and_gc() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Count",
            "tenant-kvi-cnt"
        );
        let mut a = alloc(tenant);
        let id1 = a.allocate(&ls(&[("app", "a")]), 0).unwrap();
        let _ = a.allocate(&ls(&[("app", "b")]), 0).unwrap();
        a.release(id1, 0).unwrap();
        let _ = a.gc(70 * 1_000_000_000);
        assert_eq!(a.count(), 1);
    }

    #[test]
    fn refcount_unknown_returns_zero() {
        let (_c, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "Refcount.NotFound",
            "tenant-kvi-rcnf"
        );
        let a = alloc(tenant);
        assert_eq!(a.refcount(MIN_GLOBAL_IDENTITY + 99), 0);
    }

    // ── Serde ───────────────────────────────────────────────────────────────

    #[test]
    fn global_identity_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!(
            "pkg/identity/cache/kvstore_allocator.go",
            "GlobalIdentity.Serde",
            "tenant-kvi-serde"
        );
        let g = GlobalIdentity {
            identity: MIN_GLOBAL_IDENTITY + 10,
            labels: ls(&[("app", "web")]),
            refcount: 2,
            released_at_ns: None,
        };
        let s = serde_json::to_string(&g).unwrap();
        let back: GlobalIdentity = serde_json::from_str(&s).unwrap();
        assert_eq!(back, g);
    }
}
