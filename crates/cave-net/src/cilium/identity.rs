// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cilium numeric identity allocator.
//!
//! Mirrors `pkg/identity/cache/local.go::LocalIdentityCache` plus the
//! reserved-identity table from `pkg/identity/numericidentity.go`. Each
//! unique normalised label set gets a numeric ID assigned the first time
//! it is seen; the same label set always returns the same ID.
//!
//! Reserved IDs (1..256) are the upstream well-known identities (`host`,
//! `world`, `unmanaged`, …). User identities start at
//! [`MIN_LOCAL_IDENTITY`] = 256.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lowest numeric identity that the local allocator may issue. Mirrors
/// `MinimalAllocationIdentity` in upstream.
pub const MIN_LOCAL_IDENTITY: u32 = 256;

/// Highest allocatable identity (24-bit space minus reserved upper range).
/// Mirrors `MaximalAllocationIdentity` upstream.
pub const MAX_LOCAL_IDENTITY: u32 = (1 << 24) - 1;

/// A normalised label set, modelled as sorted (key, value) pairs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LabelSet {
    pub pairs: Vec<(String, String)>,
}

impl LabelSet {
    /// Construct from any iterator of (key, value), sorting and deduplicating
    /// so two semantically-equal sets compare equal regardless of input
    /// order. Mirrors `labels.Labels.Sort`.
    pub fn from_iter<I, K, V>(it: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut pairs: Vec<(String, String)> =
            it.into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        pairs.sort();
        pairs.dedup_by(|a, b| a.0 == b.0); // duplicate keys → first wins
        Self { pairs }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("label set is empty")]
    EmptyLabels,
    #[error("identity space exhausted (reached {0})")]
    Exhausted(u32),
    #[error("tenant {tenant} cannot resolve identity owned by another tenant")]
    TenantDenied { tenant: TenantId },
    #[error("reserved identity {0} cannot be allocated by the local cache")]
    ReservedIdentity(u32),
    #[error("label set must contain reserved label `reserved:{0}` to claim that identity")]
    MissingReservedLabel(&'static str),
}

/// Reserved (well-known) identity numbers from `pkg/identity/numericidentity.go`.
/// Only a few are exported here; the rest live in upstream constants.
pub const ID_HOST: u32 = 1;
pub const ID_WORLD: u32 = 2;
pub const ID_UNMANAGED: u32 = 3;
pub const ID_HEALTH: u32 = 4;
pub const ID_INIT: u32 = 5;
pub const ID_REMOTE_NODE: u32 = 6;

/// Returns the reserved identity for a label set if exactly one
/// `reserved:<name>` label is present and corresponds to a known identity.
/// Mirrors `getReservedID` in upstream.
pub fn reserved_identity_for(labels: &LabelSet) -> Option<u32> {
    let mut reserved: Option<&str> = None;
    for (k, v) in &labels.pairs {
        if k == "reserved" {
            if reserved.is_some() {
                return None; // ambiguous
            }
            reserved = Some(v.as_str());
        }
    }
    match reserved? {
        "host" => Some(ID_HOST),
        "world" => Some(ID_WORLD),
        "unmanaged" => Some(ID_UNMANAGED),
        "health" => Some(ID_HEALTH),
        "init" => Some(ID_INIT),
        "remote-node" => Some(ID_REMOTE_NODE),
        _ => None,
    }
}

/// In-memory tenant-scoped identity cache. Mirrors `LocalIdentityCache`
/// in upstream; the `tenant` field is a cave-specific addition.
#[derive(Debug)]
pub struct LocalIdentityCache {
    pub tenant: TenantId,
    next_id: u32,
    by_labels: HashMap<LabelSet, u32>,
    by_id: HashMap<u32, LabelSet>,
}

impl LocalIdentityCache {
    pub fn new(tenant: TenantId) -> Self {
        Self {
            tenant,
            next_id: MIN_LOCAL_IDENTITY,
            by_labels: HashMap::new(),
            by_id: HashMap::new(),
        }
    }

    /// Allocate-or-lookup. Idempotent: same label set → same numeric id.
    /// Reserved label sets resolve to their reserved ID without consuming
    /// a local slot.
    pub fn lookup_or_allocate(&mut self, labels: &LabelSet) -> Result<u32, IdentityError> {
        if labels.pairs.is_empty() {
            return Err(IdentityError::EmptyLabels);
        }
        if let Some(rid) = reserved_identity_for(labels) {
            return Ok(rid);
        }
        if let Some(&id) = self.by_labels.get(labels) {
            return Ok(id);
        }
        if self.next_id > MAX_LOCAL_IDENTITY {
            return Err(IdentityError::Exhausted(self.next_id - 1));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.by_labels.insert(labels.clone(), id);
        self.by_id.insert(id, labels.clone());
        Ok(id)
    }

    /// Resolve a numeric ID back to its label set.
    pub fn lookup_by_id(&self, id: u32) -> Option<&LabelSet> {
        self.by_id.get(&id)
    }

    /// Release an identity. The slot is *not* reused (same upstream
    /// behaviour — local IDs are monotonic until the cache is rebuilt).
    pub fn release(&mut self, id: u32) -> bool {
        if let Some(labels) = self.by_id.remove(&id) {
            self.by_labels.remove(&labels);
            true
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/identity/cache/local.go", "LocalIdentityCache");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    fn ls(pairs: &[(&str, &str)]) -> LabelSet {
        LabelSet::from_iter(pairs.iter().map(|(k, v)| (*k, *v)))
    }

    #[test]
    fn label_set_normalises_input_order() {
        let (_cite, _t) = cilium_test_ctx!(
            "pkg/labels/labels.go",
            "Labels.Sort",
            "tenant-id-norm"
        );
        let a = ls(&[("app", "web"), ("env", "prod")]);
        let b = ls(&[("env", "prod"), ("app", "web")]);
        assert_eq!(a, b);
    }

    #[test]
    fn first_allocation_starts_at_min_local_identity() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "LocalIdentityCache.lookupOrCreate",
            "tenant-id-first"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let id = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
        assert_eq!(id, MIN_LOCAL_IDENTITY);
    }

    #[test]
    fn same_label_set_returns_same_identity() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "lookupOrCreate",
            "tenant-id-stable"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let a = cache.lookup_or_allocate(&ls(&[("app", "web"), ("env", "prod")])).unwrap();
        let b = cache.lookup_or_allocate(&ls(&[("env", "prod"), ("app", "web")])).unwrap();
        assert_eq!(a, b);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn distinct_label_sets_get_distinct_ids() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "LocalIdentityCache",
            "tenant-id-distinct"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let a = cache.lookup_or_allocate(&ls(&[("app", "web")])).unwrap();
        let b = cache.lookup_or_allocate(&ls(&[("app", "api")])).unwrap();
        assert_ne!(a, b);
        assert_eq!(b, a + 1);
    }

    #[test]
    fn reserved_label_resolves_to_reserved_id() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/numericidentity.go",
            "GetReservedID",
            "tenant-id-reserved"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let id = cache.lookup_or_allocate(&ls(&[("reserved", "host")])).unwrap();
        assert_eq!(id, ID_HOST);
        // Reserved lookups do not consume a local slot.
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn lookup_by_id_round_trips() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "LocalIdentityCache.LookupByID",
            "tenant-id-rev"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let labels = ls(&[("app", "web")]);
        let id = cache.lookup_or_allocate(&labels).unwrap();
        assert_eq!(cache.lookup_by_id(id).unwrap(), &labels);
    }

    #[test]
    fn release_drops_an_identity_and_does_not_reuse_slot() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "LocalIdentityCache.Release",
            "tenant-id-release"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let id_a = cache.lookup_or_allocate(&ls(&[("app", "a")])).unwrap();
        let id_b = cache.lookup_or_allocate(&ls(&[("app", "b")])).unwrap();
        assert!(cache.release(id_a));
        // Re-allocating `a` does NOT reuse `id_a` — it gets a new monotonic id.
        let id_a2 = cache.lookup_or_allocate(&ls(&[("app", "a")])).unwrap();
        assert_eq!(id_a2, id_b + 1);
        assert!(!cache.release(99_999));
    }

    #[test]
    fn empty_label_set_is_rejected() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/cache/local.go",
            "lookupOrCreate",
            "tenant-id-empty"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        let err = cache.lookup_or_allocate(&LabelSet { pairs: vec![] }).unwrap_err();
        assert!(matches!(err, IdentityError::EmptyLabels));
    }

    #[test]
    fn reserved_label_with_unknown_name_falls_through_to_local_alloc() {
        let (_cite, tenant) = cilium_test_ctx!(
            "pkg/identity/numericidentity.go",
            "GetReservedID",
            "tenant-id-unknown-reserved"
        );
        let mut cache = LocalIdentityCache::new(tenant);
        // `reserved:custom` is not a known reserved name → local allocation.
        let id = cache.lookup_or_allocate(&ls(&[("reserved", "custom")])).unwrap();
        assert!(id >= MIN_LOCAL_IDENTITY);
    }
}
