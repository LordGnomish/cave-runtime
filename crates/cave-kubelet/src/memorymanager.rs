// SPDX-License-Identifier: AGPL-3.0-or-later
//! Memory manager — Single-NUMA-node policy with reserved memory.
//!
//! Mirrors `pkg/kubelet/cm/memorymanager`: per-NUMA-node accounting of
//! memory + hugepages, allocation that pins each Guaranteed pod to a
//! single NUMA node when `--memory-manager-policy=Static`, and reserved
//! system memory carve-out via `--reserved-memory`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MemoryResource {
    Memory,
    Hugepages2Mi,
    Hugepages1Gi,
}

impl MemoryResource {
    pub fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Hugepages2Mi => "hugepages-2Mi",
            Self::Hugepages1Gi => "hugepages-1Gi",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumaMemoryBlock {
    pub numa_node: i64,
    pub by_resource: BTreeMap<MemoryResource, u64>,
}

impl NumaMemoryBlock {
    pub fn new(numa_node: i64) -> Self {
        Self {
            numa_node,
            by_resource: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, r: MemoryResource, bytes: u64) {
        self.by_resource.insert(r, bytes);
    }

    pub fn get(&self, r: MemoryResource) -> u64 {
        self.by_resource.get(&r).copied().unwrap_or(0)
    }

    pub fn add(&mut self, r: MemoryResource, bytes: u64) {
        let cur = self.get(r);
        self.by_resource.insert(r, cur.saturating_add(bytes));
    }

    pub fn sub(&mut self, r: MemoryResource, bytes: u64) {
        let cur = self.get(r);
        self.by_resource.insert(r, cur.saturating_sub(bytes));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryManagerPolicy {
    /// No NUMA pinning.
    None,
    /// Single-NUMA-node static policy: each Guaranteed pod's memory must
    /// fit in one NUMA node.
    Static,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub memory_bytes: u64,
    pub hugepages_2mi_bytes: u64,
    pub hugepages_1gi_bytes: u64,
}

impl ResourceRequest {
    pub fn for_resource(&self, r: MemoryResource) -> u64 {
        match r {
            MemoryResource::Memory => self.memory_bytes,
            MemoryResource::Hugepages2Mi => self.hugepages_2mi_bytes,
            MemoryResource::Hugepages1Gi => self.hugepages_1gi_bytes,
        }
    }

    pub fn nonzero_resources(&self) -> Vec<MemoryResource> {
        let mut v = Vec::new();
        if self.memory_bytes > 0 {
            v.push(MemoryResource::Memory);
        }
        if self.hugepages_2mi_bytes > 0 {
            v.push(MemoryResource::Hugepages2Mi);
        }
        if self.hugepages_1gi_bytes > 0 {
            v.push(MemoryResource::Hugepages1Gi);
        }
        v
    }

    pub fn is_zero(&self) -> bool {
        self.memory_bytes == 0
            && self.hugepages_2mi_bytes == 0
            && self.hugepages_1gi_bytes == 0
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemoryError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("insufficient: need {need} {resource}, have {have}")]
    Insufficient {
        resource: String,
        need: u64,
        have: u64,
    },
    #[error("conflict: {0}")]
    Conflict(String),
}

pub type MemResult<T> = Result<T, MemoryError>;

#[derive(Debug, Clone)]
pub struct MemoryAssignment {
    pub pod_uid: String,
    pub container: String,
    pub numa_node: i64,
    pub request: ResourceRequest,
}

#[derive(Debug)]
pub struct MemoryManager {
    pub policy: MemoryManagerPolicy,
    /// Total per-NUMA capacity (system view).
    capacity: BTreeMap<i64, NumaMemoryBlock>,
    /// Reserved-for-system memory per NUMA.
    reserved: BTreeMap<i64, NumaMemoryBlock>,
    /// Allocated memory per NUMA (sum of pod assignments).
    allocated: BTreeMap<i64, NumaMemoryBlock>,
    assignments: Vec<MemoryAssignment>,
}

impl MemoryManager {
    pub fn new(policy: MemoryManagerPolicy) -> Self {
        Self {
            policy,
            capacity: BTreeMap::new(),
            reserved: BTreeMap::new(),
            allocated: BTreeMap::new(),
            assignments: Vec::new(),
        }
    }

    pub fn add_numa_node(&mut self, numa_node: i64, capacity: NumaMemoryBlock) {
        self.capacity.insert(numa_node, capacity);
        self.allocated
            .entry(numa_node)
            .or_insert_with(|| NumaMemoryBlock::new(numa_node));
        self.reserved
            .entry(numa_node)
            .or_insert_with(|| NumaMemoryBlock::new(numa_node));
    }

    pub fn set_reserved(&mut self, numa_node: i64, reserved: NumaMemoryBlock) -> MemResult<()> {
        if !self.capacity.contains_key(&numa_node) {
            return Err(MemoryError::Invalid(format!(
                "numa node {} unknown",
                numa_node
            )));
        }
        let cap = self.capacity.get(&numa_node).unwrap();
        for (r, bytes) in &reserved.by_resource {
            if cap.get(*r) < *bytes {
                return Err(MemoryError::Invalid(format!(
                    "reserved {} > capacity for {}",
                    bytes,
                    r.name()
                )));
            }
        }
        self.reserved.insert(numa_node, reserved);
        Ok(())
    }

    pub fn numa_nodes(&self) -> Vec<i64> {
        self.capacity.keys().copied().collect()
    }

    pub fn allocatable(&self, numa: i64, r: MemoryResource) -> u64 {
        let cap = self.capacity.get(&numa).map(|b| b.get(r)).unwrap_or(0);
        let rsv = self.reserved.get(&numa).map(|b| b.get(r)).unwrap_or(0);
        cap.saturating_sub(rsv)
    }

    pub fn available(&self, numa: i64, r: MemoryResource) -> u64 {
        let alloc = self.allocated.get(&numa).map(|b| b.get(r)).unwrap_or(0);
        self.allocatable(numa, r).saturating_sub(alloc)
    }

    pub fn allocated_on(&self, numa: i64, r: MemoryResource) -> u64 {
        self.allocated.get(&numa).map(|b| b.get(r)).unwrap_or(0)
    }

    /// Allocate a pod's memory request to a single NUMA node when policy=Static.
    /// Picks the NUMA with the most available memory that satisfies the request.
    pub fn allocate(
        &mut self,
        pod_uid: &str,
        container: &str,
        request: ResourceRequest,
        guaranteed_integer: bool,
    ) -> MemResult<Option<i64>> {
        if matches!(self.policy, MemoryManagerPolicy::None) || request.is_zero() {
            return Ok(None);
        }
        if !guaranteed_integer {
            return Ok(None);
        }
        // Already allocated?
        if let Some(existing) = self
            .assignments
            .iter()
            .find(|a| a.pod_uid == pod_uid && a.container == container)
        {
            if existing.request == request {
                return Ok(Some(existing.numa_node));
            }
            return Err(MemoryError::Conflict(format!(
                "pod {} container {} already allocated, request differs",
                pod_uid, container
            )));
        }
        // Find best fit: NUMA where every nonzero resource fits.
        let resources = request.nonzero_resources();
        let mut best: Option<(i64, u64)> = None;
        let mut numas: Vec<i64> = self.capacity.keys().copied().collect();
        numas.sort();
        for numa in &numas {
            let mut fits = true;
            let mut headroom: u64 = 0;
            for r in &resources {
                let need = request.for_resource(*r);
                let have = self.available(*numa, *r);
                if have < need {
                    fits = false;
                    break;
                }
                headroom = headroom.saturating_add(have - need);
            }
            if fits {
                match best {
                    None => best = Some((*numa, headroom)),
                    Some((_, h)) if headroom > h => best = Some((*numa, headroom)),
                    _ => {}
                }
            }
        }
        match best {
            None => {
                let (resource_name, need) = resources
                    .first()
                    .map(|r| (r.name().to_string(), request.for_resource(*r)))
                    .unwrap_or(("memory".to_string(), 0));
                Err(MemoryError::Insufficient {
                    resource: resource_name,
                    need,
                    have: 0,
                })
            }
            Some((numa, _)) => {
                let block = self.allocated.get_mut(&numa).unwrap();
                for r in &resources {
                    block.add(*r, request.for_resource(*r));
                }
                self.assignments.push(MemoryAssignment {
                    pod_uid: pod_uid.into(),
                    container: container.into(),
                    numa_node: numa,
                    request,
                });
                Ok(Some(numa))
            }
        }
    }

    pub fn deallocate(&mut self, pod_uid: &str, container: &str) {
        let pos = self
            .assignments
            .iter()
            .position(|a| a.pod_uid == pod_uid && a.container == container);
        if let Some(idx) = pos {
            let a = self.assignments.remove(idx);
            if let Some(b) = self.allocated.get_mut(&a.numa_node) {
                for r in a.request.nonzero_resources() {
                    b.sub(r, a.request.for_resource(r));
                }
            }
        }
    }

    pub fn assignment(&self, pod_uid: &str, container: &str) -> Option<&MemoryAssignment> {
        self.assignments
            .iter()
            .find(|a| a.pod_uid == pod_uid && a.container == container)
    }

    pub fn assignments_count(&self) -> usize {
        self.assignments.len()
    }
}

/// Validate that a memory request is admissible somewhere given the
/// current snapshot of available memory across NUMA nodes — used at
/// admission time before allocation.
pub fn can_be_admitted(
    request: &ResourceRequest,
    capacity_per_numa: &BTreeMap<i64, NumaMemoryBlock>,
    reserved_per_numa: &BTreeMap<i64, NumaMemoryBlock>,
) -> bool {
    if request.is_zero() {
        return true;
    }
    for (numa, cap) in capacity_per_numa {
        let rsv = reserved_per_numa
            .get(numa)
            .cloned()
            .unwrap_or_else(|| NumaMemoryBlock::new(*numa));
        let mut fits = true;
        for r in request.nonzero_resources() {
            let need = request.for_resource(r);
            let avail = cap.get(r).saturating_sub(rsv.get(r));
            if avail < need {
                fits = false;
                break;
            }
        }
        if fits {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(numa: i64, mem: u64, hp2: u64, hp1: u64) -> NumaMemoryBlock {
        let mut b = NumaMemoryBlock::new(numa);
        b.set(MemoryResource::Memory, mem);
        b.set(MemoryResource::Hugepages2Mi, hp2);
        b.set(MemoryResource::Hugepages1Gi, hp1);
        b
    }

    fn req(mem: u64, hp2: u64, hp1: u64) -> ResourceRequest {
        ResourceRequest {
            memory_bytes: mem,
            hugepages_2mi_bytes: hp2,
            hugepages_1gi_bytes: hp1,
        }
    }

    fn mgr_2numa() -> MemoryManager {
        let mut m = MemoryManager::new(MemoryManagerPolicy::Static);
        m.add_numa_node(0, block(0, 8 * 1024 * 1024 * 1024, 0, 0));
        m.add_numa_node(1, block(1, 8 * 1024 * 1024 * 1024, 0, 0));
        m
    }

    #[test]
    fn memory_resource_names() {
        assert_eq!(MemoryResource::Memory.name(), "memory");
        assert_eq!(MemoryResource::Hugepages2Mi.name(), "hugepages-2Mi");
        assert_eq!(MemoryResource::Hugepages1Gi.name(), "hugepages-1Gi");
    }

    #[test]
    fn block_set_get_add_sub() {
        let mut b = NumaMemoryBlock::new(0);
        b.set(MemoryResource::Memory, 100);
        assert_eq!(b.get(MemoryResource::Memory), 100);
        b.add(MemoryResource::Memory, 50);
        assert_eq!(b.get(MemoryResource::Memory), 150);
        b.sub(MemoryResource::Memory, 70);
        assert_eq!(b.get(MemoryResource::Memory), 80);
    }

    #[test]
    fn block_sub_saturates_at_zero() {
        let mut b = NumaMemoryBlock::new(0);
        b.set(MemoryResource::Memory, 50);
        b.sub(MemoryResource::Memory, 100);
        assert_eq!(b.get(MemoryResource::Memory), 0);
    }

    #[test]
    fn manager_construction_with_nodes() {
        let m = mgr_2numa();
        assert_eq!(m.numa_nodes(), vec![0, 1]);
    }

    #[test]
    fn manager_set_reserved_validates_node_exists() {
        let mut m = MemoryManager::new(MemoryManagerPolicy::Static);
        let res = m.set_reserved(99, block(99, 0, 0, 0));
        assert!(res.is_err());
    }

    #[test]
    fn manager_set_reserved_rejects_more_than_capacity() {
        let mut m = mgr_2numa();
        let mut r = NumaMemoryBlock::new(0);
        r.set(MemoryResource::Memory, 1024 * 1024 * 1024 * 1024);
        assert!(m.set_reserved(0, r).is_err());
    }

    #[test]
    fn manager_allocatable_subtracts_reserved() {
        let mut m = mgr_2numa();
        m.set_reserved(0, block(0, 1024 * 1024 * 1024, 0, 0)).unwrap();
        assert_eq!(
            m.allocatable(0, MemoryResource::Memory),
            7 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn manager_available_minus_allocated() {
        let mut m = mgr_2numa();
        m.allocate("p", "c", req(1024 * 1024 * 1024, 0, 0), true).unwrap();
        assert_eq!(
            m.available(0, MemoryResource::Memory),
            7 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn allocate_none_policy_returns_none() {
        let mut m = MemoryManager::new(MemoryManagerPolicy::None);
        m.add_numa_node(0, block(0, 1024, 0, 0));
        let res = m.allocate("p", "c", req(512, 0, 0), true).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn allocate_zero_request_returns_none() {
        let mut m = mgr_2numa();
        let res = m.allocate("p", "c", req(0, 0, 0), true).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn allocate_non_guaranteed_returns_none() {
        let mut m = mgr_2numa();
        let res = m.allocate("p", "c", req(1024, 0, 0), false).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn allocate_picks_a_numa_with_capacity() {
        let mut m = mgr_2numa();
        let n = m.allocate("p", "c", req(1024, 0, 0), true).unwrap();
        assert!(n.is_some());
        assert!(m.numa_nodes().contains(&n.unwrap()));
    }

    #[test]
    fn allocate_two_pods_balance_across_numa() {
        let mut m = mgr_2numa();
        let _ = m.allocate("p1", "c", req(7 * 1024 * 1024 * 1024, 0, 0), true).unwrap();
        // Second allocation must go to the other NUMA — first is mostly full.
        let n2 = m.allocate("p2", "c", req(7 * 1024 * 1024 * 1024, 0, 0), true).unwrap().unwrap();
        let assignments: Vec<i64> = m.assignments.iter().map(|a| a.numa_node).collect();
        assert!(assignments.contains(&n2));
        assert_eq!(assignments.iter().collect::<std::collections::BTreeSet<_>>().len(), 2);
    }

    #[test]
    fn allocate_idempotent_for_same_request() {
        let mut m = mgr_2numa();
        let n1 = m.allocate("p", "c", req(1024, 0, 0), true).unwrap();
        let n2 = m.allocate("p", "c", req(1024, 0, 0), true).unwrap();
        assert_eq!(n1, n2);
    }

    #[test]
    fn allocate_conflict_when_request_differs() {
        let mut m = mgr_2numa();
        m.allocate("p", "c", req(1024, 0, 0), true).unwrap();
        let err = m.allocate("p", "c", req(2048, 0, 0), true).unwrap_err();
        assert!(matches!(err, MemoryError::Conflict(_)));
    }

    #[test]
    fn allocate_insufficient_when_no_numa_fits() {
        let mut m = MemoryManager::new(MemoryManagerPolicy::Static);
        m.add_numa_node(0, block(0, 1024, 0, 0));
        let err = m.allocate("p", "c", req(2048, 0, 0), true).unwrap_err();
        assert!(matches!(err, MemoryError::Insufficient { .. }));
    }

    #[test]
    fn allocate_requires_single_numa_for_combined_resources() {
        // 8 GiB memory on NUMA 0 only, hugepages on NUMA 1 only.
        // Single-NUMA policy: cannot satisfy mixed request.
        let mut m = MemoryManager::new(MemoryManagerPolicy::Static);
        m.add_numa_node(0, block(0, 8 * 1024 * 1024 * 1024, 0, 0));
        m.add_numa_node(1, block(1, 0, 4 * 1024 * 1024 * 1024, 0));
        let err = m.allocate("p", "c", req(1024, 1024, 0), true).unwrap_err();
        assert!(matches!(err, MemoryError::Insufficient { .. }));
    }

    #[test]
    fn deallocate_releases_memory() {
        let mut m = mgr_2numa();
        let n = m.allocate("p", "c", req(1024 * 1024, 0, 0), true).unwrap().unwrap();
        m.deallocate("p", "c");
        assert_eq!(m.allocated_on(n, MemoryResource::Memory), 0);
        assert!(m.assignment("p", "c").is_none());
    }

    #[test]
    fn deallocate_unknown_is_noop() {
        let mut m = mgr_2numa();
        m.deallocate("ghost", "ghost");
    }

    #[test]
    fn assignment_lookup_returns_record() {
        let mut m = mgr_2numa();
        m.allocate("p", "c", req(1024, 0, 0), true).unwrap();
        let a = m.assignment("p", "c").unwrap();
        assert_eq!(a.pod_uid, "p");
    }

    #[test]
    fn assignments_count_growth() {
        let mut m = mgr_2numa();
        m.allocate("p1", "c", req(1024, 0, 0), true).unwrap();
        m.allocate("p2", "c", req(1024, 0, 0), true).unwrap();
        assert_eq!(m.assignments_count(), 2);
    }

    #[test]
    fn can_be_admitted_zero_request_always_ok() {
        let cap: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        let rsv: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        assert!(can_be_admitted(&req(0, 0, 0), &cap, &rsv));
    }

    #[test]
    fn can_be_admitted_passes_when_one_numa_fits() {
        let mut cap: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        cap.insert(0, block(0, 1024, 0, 0));
        cap.insert(1, block(1, 8192, 0, 0));
        let rsv: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        assert!(can_be_admitted(&req(2048, 0, 0), &cap, &rsv));
    }

    #[test]
    fn can_be_admitted_fails_when_no_numa_fits() {
        let mut cap: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        cap.insert(0, block(0, 1024, 0, 0));
        cap.insert(1, block(1, 1024, 0, 0));
        let rsv: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        assert!(!can_be_admitted(&req(2048, 0, 0), &cap, &rsv));
    }

    #[test]
    fn can_be_admitted_respects_reserved() {
        let mut cap: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        cap.insert(0, block(0, 1024, 0, 0));
        let mut rsv: BTreeMap<i64, NumaMemoryBlock> = BTreeMap::new();
        rsv.insert(0, block(0, 800, 0, 0));
        assert!(!can_be_admitted(&req(500, 0, 0), &cap, &rsv));
        assert!(can_be_admitted(&req(200, 0, 0), &cap, &rsv));
    }

    #[test]
    fn resource_request_nonzero_resources() {
        let r = req(100, 0, 200);
        assert_eq!(
            r.nonzero_resources(),
            vec![MemoryResource::Memory, MemoryResource::Hugepages1Gi]
        );
    }

    #[test]
    fn resource_request_is_zero() {
        assert!(req(0, 0, 0).is_zero());
        assert!(!req(1, 0, 0).is_zero());
    }

    #[test]
    fn allocate_with_hugepages_only() {
        let mut m = MemoryManager::new(MemoryManagerPolicy::Static);
        let mut b = NumaMemoryBlock::new(0);
        b.set(MemoryResource::Hugepages2Mi, 1024 * 1024 * 1024);
        m.add_numa_node(0, b);
        let n = m.allocate("p", "c", req(0, 1024 * 1024, 0), true).unwrap();
        assert!(n.is_some());
    }

    #[test]
    fn allocated_on_zero_for_unused_numa() {
        let m = mgr_2numa();
        assert_eq!(m.allocated_on(0, MemoryResource::Memory), 0);
    }

    #[test]
    fn allocate_picks_numa_with_most_headroom() {
        let mut m = MemoryManager::new(MemoryManagerPolicy::Static);
        m.add_numa_node(0, block(0, 1 * 1024 * 1024 * 1024, 0, 0));
        m.add_numa_node(1, block(1, 16 * 1024 * 1024 * 1024, 0, 0));
        let n = m.allocate("p", "c", req(512 * 1024 * 1024, 0, 0), true).unwrap().unwrap();
        // Should pick numa 1 (more headroom).
        assert_eq!(n, 1);
    }
}
