// SPDX-License-Identifier: AGPL-3.0-or-later
//! Generic key→ID allocator.
//!
//! Mirrors `pkg/allocator/allocator.go`. Cilium's allocator is the
//! abstraction used by the identity subsystem (and others) to claim
//! unique numeric IDs for opaque keys, with a kvstore-backed
//! coordination plane.
//!
//! We port the in-memory model:
//!   * monotonically increasing ID allocation within a configured range
//!   * idempotent allocation (same key returns the same ID)
//!   * release & re-allocation behaviour
//!   * tenant-scoped state

use crate::cilium::types::{Cite, TenantId};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AllocError {
    #[error("allocator exhausted: id {id} would exceed max {max}")]
    Exhausted { id: u64, max: u64 },
    #[error("id {0} is reserved (must be inside [min, max])")]
    OutOfRange(u64),
    #[error("tenant {tenant} cannot mutate allocator owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Generic allocator. `K` is the opaque key type (for identities the upstream
/// uses a label-set hash; we keep it generic).
#[derive(Debug)]
pub struct Allocator<K: Ord + Clone> {
    pub tenant: TenantId,
    pub min: u64,
    pub max: u64,
    next: u64,
    by_key: BTreeMap<K, u64>,
    by_id: BTreeMap<u64, K>,
}

impl<K: Ord + Clone> Allocator<K> {
    pub fn new(tenant: TenantId, min: u64, max: u64) -> Self {
        assert!(min <= max);
        Self {
            tenant, min, max, next: min,
            by_key: BTreeMap::new(),
            by_id: BTreeMap::new(),
        }
    }

    /// Allocate (or return the existing ID for) `key`. Idempotent.
    pub fn allocate(&mut self, key: K) -> Result<u64, AllocError> {
        if let Some(&id) = self.by_key.get(&key) {
            return Ok(id);
        }
        // find next free id starting from self.next, scanning forward
        while self.next <= self.max && self.by_id.contains_key(&self.next) {
            self.next += 1;
        }
        if self.next > self.max {
            return Err(AllocError::Exhausted { id: self.next, max: self.max });
        }
        let id = self.next;
        self.next += 1;
        self.by_key.insert(key.clone(), id);
        self.by_id.insert(id, key);
        Ok(id)
    }

    pub fn lookup_id(&self, key: &K) -> Option<u64> { self.by_key.get(key).copied() }
    pub fn lookup_key(&self, id: u64) -> Option<&K> { self.by_id.get(&id) }

    pub fn release(&mut self, key: &K) -> Option<u64> {
        let id = self.by_key.remove(key)?;
        self.by_id.remove(&id);
        if id < self.next { self.next = id; }
        Some(id)
    }

    pub fn len(&self) -> usize { self.by_key.len() }
    pub fn is_empty(&self) -> bool { self.by_key.is_empty() }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/allocator/allocator.go", "Allocator");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn allocator_allocates_within_range() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Range", "tenant-al-r");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        let id = a.allocate("k1".into()).unwrap();
        assert!(id >= 1024 && id <= 65535);
    }

    #[test]
    fn allocator_is_idempotent_per_key() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Idempotent", "tenant-al-id");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        let a1 = a.allocate("k1".into()).unwrap();
        let a2 = a.allocate("k1".into()).unwrap();
        assert_eq!(a1, a2);
        assert_eq!(a.len(), 1);
    }

    #[test]
    fn allocator_returns_distinct_ids_for_distinct_keys() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Distinct", "tenant-al-d");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        let id1 = a.allocate("k1".into()).unwrap();
        let id2 = a.allocate("k2".into()).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn allocator_starts_at_min() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "MinStart", "tenant-al-ms");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        let id = a.allocate("k1".into()).unwrap();
        assert_eq!(id, 1024);
    }

    #[test]
    fn allocator_exhaustion_returns_error() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Exhaust", "tenant-al-ex");
        let mut a: Allocator<String> = Allocator::new(t, 1, 2);
        a.allocate("k1".into()).unwrap();
        a.allocate("k2".into()).unwrap();
        let e = a.allocate("k3".into()).unwrap_err();
        assert!(matches!(e, AllocError::Exhausted { .. }));
    }

    #[test]
    fn allocator_release_frees_id_for_reuse() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "ReleaseReuse", "tenant-al-rr");
        let mut a: Allocator<String> = Allocator::new(t, 1, 2);
        let id1 = a.allocate("k1".into()).unwrap();
        a.release(&"k1".into());
        let id2 = a.allocate("k2".into()).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn allocator_lookup_id_and_key() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Lookup", "tenant-al-lk");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        let id = a.allocate("hello".into()).unwrap();
        assert_eq!(a.lookup_id(&"hello".into()), Some(id));
        assert_eq!(a.lookup_key(id), Some(&"hello".to_string()));
    }

    #[test]
    fn allocator_release_returns_freed_id() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "ReleaseId", "tenant-al-rid");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        let id = a.allocate("k1".into()).unwrap();
        let freed = a.release(&"k1".into()).unwrap();
        assert_eq!(freed, id);
    }

    #[test]
    fn allocator_release_unknown_returns_none() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Release.Miss", "tenant-al-rm");
        let mut a: Allocator<String> = Allocator::new(t, 1024, 65535);
        assert!(a.release(&"ghost".into()).is_none());
    }

    #[test]
    fn allocator_empty_state_reports_zero() {
        let (_c, t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Empty", "tenant-al-e");
        let a: Allocator<String> = Allocator::new(t, 1024, 65535);
        assert!(a.is_empty());
        assert_eq!(a.len(), 0);
    }

    #[test]
    fn allocator_error_renders() {
        let (_c, _t) = cilium_test_ctx!("pkg/allocator/allocator.go", "Errors", "tenant-al-er");
        let e = AllocError::Exhausted { id: 100, max: 99 };
        assert!(format!("{}", e).contains("99"));
        let e = AllocError::OutOfRange(0);
        assert!(format!("{}", e).contains("reserved"));
    }
}
