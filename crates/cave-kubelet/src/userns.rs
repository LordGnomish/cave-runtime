// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! User-namespace remapping — KEP-127 (v1.30 beta).
//!
//! Mirrors `pkg/kubelet/userns/` from upstream. Computes the
//! per-pod uid/gid map ranges that a pod runs with when
//! `hostUsers: false` is set on the pod spec. The actual
//! `unshare(CLONE_NEWUSER)` + `/proc/<pid>/uid_map` writes
//! live in the CRI runtime; this module owns the *allocation*
//! state machine.
//!
//! Upstream policy:
//!
//! * Reserve a fixed-size range on the host (default 65 536
//!   ids per pod).
//! * Allocate from a free-list, never overlap two pods.
//! * Track per-pod assignment so re-startups get the same
//!   range (id stability across container restarts).

use std::collections::BTreeMap;

/// Number of uids/gids one pod gets — Kubernetes default
/// matches upstream constant `userNsSize = 65536`.
pub const POD_RANGE_SIZE: u32 = 65_536;

/// First host uid where pod ranges start being allocated.
/// Upstream `userNsHostBase = 100_000`.
pub const HOST_BASE: u32 = 100_000;

/// Last allocatable host uid — upstream stops at 2^31 to keep
/// 32-bit-uid compatibility. Caller can override.
pub const HOST_END: u32 = 2_147_483_648;

/// One allocated range — host id `[start, start + size)` maps
/// to pod-internal `[0, size)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdRange {
    pub host_start: u32,
    pub size: u32,
}

impl IdRange {
    pub fn host_end_exclusive(&self) -> u32 {
        self.host_start.saturating_add(self.size)
    }

    pub fn overlaps(&self, other: &IdRange) -> bool {
        let a_end = self.host_end_exclusive();
        let b_end = other.host_end_exclusive();
        self.host_start < b_end && other.host_start < a_end
    }
}

/// Per-pod allocator state.
#[derive(Debug)]
pub struct UserNsAllocator {
    range_size: u32,
    next_host: u32,
    host_end: u32,
    by_pod: BTreeMap<String, IdRange>,
    /// Released ranges that can be reused before bumping
    /// `next_host`. LIFO so a quick-restart re-uses the most
    /// recent slot.
    free_list: Vec<IdRange>,
}

impl Default for UserNsAllocator {
    fn default() -> Self {
        Self::new(POD_RANGE_SIZE, HOST_BASE, HOST_END)
    }
}

impl UserNsAllocator {
    pub fn new(range_size: u32, host_base: u32, host_end: u32) -> Self {
        Self {
            range_size,
            next_host: host_base,
            host_end,
            by_pod: BTreeMap::new(),
            free_list: Vec::new(),
        }
    }

    /// Allocate a range for `pod_uid`. Idempotent — calling
    /// twice with the same uid returns the same range.
    pub fn allocate(&mut self, pod_uid: &str) -> Result<IdRange, AllocError> {
        if let Some(existing) = self.by_pod.get(pod_uid) {
            return Ok(*existing);
        }
        // Reuse a freed range if available.
        if let Some(r) = self.free_list.pop() {
            self.by_pod.insert(pod_uid.to_string(), r);
            return Ok(r);
        }
        // Allocate from the high-water mark.
        if self.next_host.saturating_add(self.range_size) > self.host_end {
            return Err(AllocError::Exhausted);
        }
        let r = IdRange {
            host_start: self.next_host,
            size: self.range_size,
        };
        self.next_host += self.range_size;
        self.by_pod.insert(pod_uid.to_string(), r);
        Ok(r)
    }

    /// Release the range a pod previously held. Idempotent.
    pub fn release(&mut self, pod_uid: &str) {
        if let Some(r) = self.by_pod.remove(pod_uid) {
            self.free_list.push(r);
        }
    }

    pub fn get(&self, pod_uid: &str) -> Option<IdRange> {
        self.by_pod.get(pod_uid).copied()
    }

    pub fn in_use(&self) -> usize {
        self.by_pod.len()
    }

    pub fn free_count(&self) -> usize {
        self.free_list.len()
    }

    /// `true` if any two assignments overlap. Should always be
    /// `false`; exposed for invariant testing.
    pub fn assignments_disjoint(&self) -> bool {
        let ranges: Vec<&IdRange> = self.by_pod.values().collect();
        for i in 0..ranges.len() {
            for j in (i + 1)..ranges.len() {
                if ranges[i].overlaps(ranges[j]) {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocError {
    /// All host uids in the configured range are taken.
    Exhausted,
}

impl std::fmt::Display for AllocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocError::Exhausted => write!(f, "user namespace host id range exhausted"),
        }
    }
}

impl std::error::Error for AllocError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_returns_distinct_ranges() {
        let mut a = UserNsAllocator::default();
        let r1 = a.allocate("pod-1").unwrap();
        let r2 = a.allocate("pod-2").unwrap();
        assert_ne!(r1.host_start, r2.host_start);
        assert_eq!(r2.host_start, r1.host_start + POD_RANGE_SIZE);
    }

    #[test]
    fn allocate_idempotent_for_same_pod() {
        let mut a = UserNsAllocator::default();
        let r1 = a.allocate("pod-1").unwrap();
        let r2 = a.allocate("pod-1").unwrap();
        assert_eq!(r1, r2);
        assert_eq!(a.in_use(), 1);
    }

    #[test]
    fn release_makes_range_reusable() {
        let mut a = UserNsAllocator::default();
        let r1 = a.allocate("pod-1").unwrap();
        a.release("pod-1");
        // Next pod re-uses the freed slot.
        let r2 = a.allocate("pod-2").unwrap();
        assert_eq!(r1, r2);
    }

    #[test]
    fn release_unknown_pod_is_noop() {
        let mut a = UserNsAllocator::default();
        a.release("never-allocated");
        assert_eq!(a.in_use(), 0);
    }

    #[test]
    fn exhausted_returns_error() {
        // Tiny allocator that holds only 2 pods.
        let mut a = UserNsAllocator::new(10, 100, 120);
        a.allocate("a").unwrap();
        a.allocate("b").unwrap();
        let err = a.allocate("c").unwrap_err();
        assert_eq!(err, AllocError::Exhausted);
    }

    #[test]
    fn assignments_disjoint_holds_after_many_allocations() {
        let mut a = UserNsAllocator::default();
        for i in 0..32 {
            a.allocate(&format!("pod-{i}")).unwrap();
        }
        assert!(a.assignments_disjoint());
    }

    #[test]
    fn id_range_overlaps_detects_overlap() {
        let a = IdRange {
            host_start: 100,
            size: 50,
        };
        let b = IdRange {
            host_start: 120,
            size: 50,
        };
        assert!(a.overlaps(&b));
        let c = IdRange {
            host_start: 200,
            size: 50,
        };
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn id_range_overlaps_handles_adjacent_as_non_overlap() {
        let a = IdRange {
            host_start: 100,
            size: 50,
        };
        let b = IdRange {
            host_start: 150,
            size: 50,
        };
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn free_list_reuses_in_lifo_order() {
        let mut a = UserNsAllocator::default();
        let r1 = a.allocate("a").unwrap();
        let r2 = a.allocate("b").unwrap();
        a.release("a");
        a.release("b");
        // LIFO — most recent release first.
        let r3 = a.allocate("c").unwrap();
        assert_eq!(r3, r2);
        let r4 = a.allocate("d").unwrap();
        assert_eq!(r4, r1);
    }

    #[test]
    fn get_returns_assigned_range() {
        let mut a = UserNsAllocator::default();
        let r = a.allocate("pod-1").unwrap();
        assert_eq!(a.get("pod-1"), Some(r));
        assert_eq!(a.get("absent"), None);
    }

    #[test]
    fn in_use_and_free_counts_track_lifecycle() {
        let mut a = UserNsAllocator::default();
        a.allocate("a").unwrap();
        a.allocate("b").unwrap();
        assert_eq!(a.in_use(), 2);
        assert_eq!(a.free_count(), 0);
        a.release("a");
        assert_eq!(a.in_use(), 1);
        assert_eq!(a.free_count(), 1);
    }
}
