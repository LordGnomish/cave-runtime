// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! CPU manager — static policy with topology-aware exclusive allocation.
//!
//! Mirrors `pkg/kubelet/cm/cpumanager`: Guaranteed-QoS pods with integer
//! CPU requests get exclusive CPUs from a per-node shared pool (minus
//! reserved CPUs for the system); other pods float in the shared pool.
//! Allocation respects topology — prefers full sockets, then full cores,
//! then NUMA-local groupings. Implements the `static` policy.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CpuSet {
    cpus: BTreeSet<i64>,
}

impl CpuSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_iter<I: IntoIterator<Item = i64>>(it: I) -> Self {
        Self {
            cpus: it.into_iter().collect(),
        }
    }

    pub fn insert(&mut self, cpu: i64) -> bool {
        self.cpus.insert(cpu)
    }

    pub fn remove(&mut self, cpu: i64) -> bool {
        self.cpus.remove(&cpu)
    }

    pub fn contains(&self, cpu: i64) -> bool {
        self.cpus.contains(&cpu)
    }

    pub fn size(&self) -> usize {
        self.cpus.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cpus.is_empty()
    }

    pub fn to_vec(&self) -> Vec<i64> {
        self.cpus.iter().copied().collect()
    }

    pub fn union(&self, other: &Self) -> Self {
        let mut out = self.clone();
        for c in &other.cpus {
            out.cpus.insert(*c);
        }
        out
    }

    pub fn intersection(&self, other: &Self) -> Self {
        Self {
            cpus: self.cpus.intersection(&other.cpus).copied().collect(),
        }
    }

    pub fn difference(&self, other: &Self) -> Self {
        Self {
            cpus: self.cpus.difference(&other.cpus).copied().collect(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &i64> {
        self.cpus.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuInfo {
    pub cpu_id: i64,
    pub core_id: i64,
    pub socket_id: i64,
    pub numa_node_id: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuTopology {
    pub cpus: BTreeMap<i64, CpuInfo>,
    pub num_cpus: usize,
    pub num_cores: usize,
    pub num_sockets: usize,
    pub num_numa_nodes: usize,
}

impl CpuTopology {
    /// Build a homogeneous topology: `sockets * cores_per_socket * threads_per_core`.
    /// CPU IDs are assigned sequentially, with hyperthread siblings paired
    /// adjacent (cpu N and N+sockets*cores share core).
    pub fn make_homogeneous(
        sockets: usize,
        cores_per_socket: usize,
        threads_per_core: usize,
    ) -> Self {
        let mut cpus = BTreeMap::new();
        let mut id = 0i64;
        for socket_id in 0..sockets {
            for core_idx in 0..cores_per_socket {
                let core_id = (socket_id * cores_per_socket + core_idx) as i64;
                for _t in 0..threads_per_core {
                    cpus.insert(
                        id,
                        CpuInfo {
                            cpu_id: id,
                            core_id,
                            socket_id: socket_id as i64,
                            numa_node_id: socket_id as i64,
                        },
                    );
                    id += 1;
                }
            }
        }
        let num_cpus = cpus.len();
        Self {
            cpus,
            num_cpus,
            num_cores: sockets * cores_per_socket,
            num_sockets: sockets,
            num_numa_nodes: sockets,
        }
    }

    pub fn cpus_in_core(&self, core_id: i64) -> CpuSet {
        CpuSet::from_iter(
            self.cpus
                .values()
                .filter(|c| c.core_id == core_id)
                .map(|c| c.cpu_id),
        )
    }

    pub fn cpus_in_socket(&self, socket_id: i64) -> CpuSet {
        CpuSet::from_iter(
            self.cpus
                .values()
                .filter(|c| c.socket_id == socket_id)
                .map(|c| c.cpu_id),
        )
    }

    pub fn cpus_in_numa(&self, numa_node: i64) -> CpuSet {
        CpuSet::from_iter(
            self.cpus
                .values()
                .filter(|c| c.numa_node_id == numa_node)
                .map(|c| c.cpu_id),
        )
    }

    pub fn cores_in_socket(&self, socket_id: i64) -> Vec<i64> {
        let mut cores: Vec<i64> = self
            .cpus
            .values()
            .filter(|c| c.socket_id == socket_id)
            .map(|c| c.core_id)
            .collect();
        cores.sort();
        cores.dedup();
        cores
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CpuManagerPolicy {
    /// No exclusive allocation; all pods share the default pool.
    None,
    /// Guaranteed pods with integer CPU get exclusive CPUs.
    Static,
}

#[derive(Debug, Default, Clone)]
pub struct CpuManagerOptions {
    pub full_pcpus_only: bool,
    pub distribute_cpus_across_numa: bool,
    pub align_by_socket: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CpuError {
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("insufficient cpus: requested {requested}, available {available}")]
    Insufficient { requested: usize, available: usize },
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub type CpuResult<T> = Result<T, CpuError>;

#[derive(Debug)]
pub struct CpuManager {
    pub topology: CpuTopology,
    pub policy: CpuManagerPolicy,
    pub options: CpuManagerOptions,
    /// CPUs reserved for system daemons (`--reserved-cpus`).
    pub reserved: CpuSet,
    /// CPUs currently in the shared (non-exclusive) pool.
    shared_pool: CpuSet,
    /// (pod_uid, container_name) → exclusively assigned CPUs.
    assignments: BTreeMap<(String, String), CpuSet>,
}

impl CpuManager {
    pub fn new(
        topology: CpuTopology,
        policy: CpuManagerPolicy,
        reserved: CpuSet,
    ) -> CpuResult<Self> {
        for r in reserved.iter() {
            if !topology.cpus.contains_key(r) {
                return Err(CpuError::Invalid(format!(
                    "reserved cpu {} not in topology",
                    r
                )));
            }
        }
        let all = CpuSet::from_iter(topology.cpus.keys().copied());
        let shared_pool = all.difference(&reserved);
        Ok(Self {
            topology,
            policy,
            options: CpuManagerOptions::default(),
            reserved,
            shared_pool,
            assignments: BTreeMap::new(),
        })
    }

    pub fn with_options(mut self, options: CpuManagerOptions) -> Self {
        self.options = options;
        self
    }

    pub fn shared_pool(&self) -> &CpuSet {
        &self.shared_pool
    }

    pub fn assignment_for(&self, pod_uid: &str, container: &str) -> Option<CpuSet> {
        self.assignments
            .get(&(pod_uid.to_string(), container.to_string()))
            .cloned()
    }

    pub fn assignments_count(&self) -> usize {
        self.assignments.len()
    }

    pub fn allocated_cpus(&self) -> CpuSet {
        let mut out = CpuSet::new();
        for s in self.assignments.values() {
            out = out.union(s);
        }
        out
    }

    /// Allocate `request` exclusive CPUs to (pod, container) using the
    /// static policy. Skips allocation if policy is None or request is 0.
    /// Idempotent for the same (pod, container).
    pub fn allocate(
        &mut self,
        pod_uid: &str,
        container: &str,
        request: usize,
        guaranteed_integer: bool,
    ) -> CpuResult<CpuSet> {
        if matches!(self.policy, CpuManagerPolicy::None) || request == 0 || !guaranteed_integer {
            return Ok(CpuSet::new());
        }
        let key = (pod_uid.to_string(), container.to_string());
        if let Some(existing) = self.assignments.get(&key) {
            if existing.size() == request {
                return Ok(existing.clone());
            }
            return Err(CpuError::Conflict(format!(
                "container {}/{} already has {} cpus, requested {}",
                pod_uid,
                container,
                existing.size(),
                request
            )));
        }
        if self.options.full_pcpus_only && !self.is_full_pcpus_aligned(request) {
            return Err(CpuError::Invalid(
                "full-pcpus-only requires request to be a multiple of threads-per-core".into(),
            ));
        }
        if self.shared_pool.size() < request {
            return Err(CpuError::Insufficient {
                requested: request,
                available: self.shared_pool.size(),
            });
        }
        let chosen = self.pick_cpus(request)?;
        for c in chosen.iter() {
            self.shared_pool.remove(*c);
        }
        self.assignments.insert(key, chosen.clone());
        Ok(chosen)
    }

    pub fn deallocate(&mut self, pod_uid: &str, container: &str) {
        let key = (pod_uid.to_string(), container.to_string());
        if let Some(set) = self.assignments.remove(&key) {
            for c in set.iter() {
                self.shared_pool.insert(*c);
            }
        }
    }

    /// Pick `n` CPUs preferring topology locality:
    ///   1. full sockets
    ///   2. full cores within the same socket
    ///   3. anywhere
    pub fn pick_cpus(&self, n: usize) -> CpuResult<CpuSet> {
        if n == 0 {
            return Ok(CpuSet::new());
        }
        let pool = self.shared_pool.clone();
        if pool.size() < n {
            return Err(CpuError::Insufficient {
                requested: n,
                available: pool.size(),
            });
        }
        let mut chosen = CpuSet::new();
        let mut remaining = pool.clone();
        // 1) Full sockets first.
        for socket in 0..self.topology.num_sockets as i64 {
            let s = self
                .topology
                .cpus_in_socket(socket)
                .intersection(&remaining);
            if s.size() == self.topology.cpus_in_socket(socket).size()
                && s.size() <= (n - chosen.size())
                && s.size() > 0
            {
                chosen = chosen.union(&s);
                remaining = remaining.difference(&s);
                if chosen.size() == n {
                    return Ok(chosen);
                }
            }
        }
        // 2) Full cores (any socket).
        let core_ids: Vec<i64> = {
            let mut v: Vec<i64> = self.topology.cpus.values().map(|c| c.core_id).collect();
            v.sort();
            v.dedup();
            v
        };
        for core in core_ids {
            let c = self.topology.cpus_in_core(core).intersection(&remaining);
            if c.size() > 0
                && c.size() == self.topology.cpus_in_core(core).size()
                && c.size() <= (n - chosen.size())
            {
                chosen = chosen.union(&c);
                remaining = remaining.difference(&c);
                if chosen.size() == n {
                    return Ok(chosen);
                }
            }
        }
        // 3) Individual CPUs from the remaining pool, lowest first.
        let mut leftovers: Vec<i64> = remaining.to_vec();
        leftovers.sort();
        for cpu in leftovers {
            if chosen.size() == n {
                break;
            }
            chosen.insert(cpu);
        }
        if chosen.size() != n {
            return Err(CpuError::Insufficient {
                requested: n,
                available: pool.size(),
            });
        }
        Ok(chosen)
    }

    fn is_full_pcpus_aligned(&self, request: usize) -> bool {
        let threads = self
            .topology
            .cpus
            .values()
            .next()
            .map(|c| self.topology.cpus_in_core(c.core_id).size())
            .unwrap_or(1);
        threads == 0 || request % threads == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topo_2s_4c_2t() -> CpuTopology {
        // 2 sockets × 4 cores × 2 threads = 16 CPUs
        CpuTopology::make_homogeneous(2, 4, 2)
    }

    #[test]
    fn cpuset_basic_ops() {
        let mut s = CpuSet::new();
        assert!(s.is_empty());
        s.insert(0);
        s.insert(2);
        assert_eq!(s.size(), 2);
        assert!(s.contains(0));
        assert!(!s.contains(1));
        s.remove(0);
        assert!(!s.contains(0));
    }

    #[test]
    fn cpuset_set_ops() {
        let a = CpuSet::from_iter(vec![0, 1, 2]);
        let b = CpuSet::from_iter(vec![1, 2, 3]);
        assert_eq!(a.union(&b), CpuSet::from_iter(vec![0, 1, 2, 3]));
        assert_eq!(a.intersection(&b), CpuSet::from_iter(vec![1, 2]));
        assert_eq!(a.difference(&b), CpuSet::from_iter(vec![0]));
    }

    #[test]
    fn topology_homogeneous_counts() {
        let t = topo_2s_4c_2t();
        assert_eq!(t.num_cpus, 16);
        assert_eq!(t.num_cores, 8);
        assert_eq!(t.num_sockets, 2);
    }

    #[test]
    fn topology_cpus_in_core_returns_threads() {
        let t = topo_2s_4c_2t();
        assert_eq!(t.cpus_in_core(0).size(), 2);
        assert_eq!(t.cpus_in_core(7).size(), 2);
    }

    #[test]
    fn topology_cpus_in_socket_correct() {
        let t = topo_2s_4c_2t();
        assert_eq!(t.cpus_in_socket(0).size(), 8);
        assert_eq!(t.cpus_in_socket(1).size(), 8);
    }

    #[test]
    fn topology_cpus_in_numa_correct() {
        let t = topo_2s_4c_2t();
        assert_eq!(t.cpus_in_numa(0).size(), 8);
        assert_eq!(t.cpus_in_numa(1).size(), 8);
    }

    #[test]
    fn topology_cores_in_socket_dedup() {
        let t = topo_2s_4c_2t();
        let cores = t.cores_in_socket(0);
        assert_eq!(cores, vec![0, 1, 2, 3]);
    }

    #[test]
    fn manager_construction_validates_reserved() {
        let t = topo_2s_4c_2t();
        let res = CpuManager::new(
            t.clone(),
            CpuManagerPolicy::Static,
            CpuSet::from_iter(vec![100]),
        );
        assert!(res.is_err());
    }

    #[test]
    fn manager_shared_pool_excludes_reserved() {
        let t = topo_2s_4c_2t();
        let m =
            CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::from_iter(vec![0, 1])).unwrap();
        assert_eq!(m.shared_pool().size(), 14);
        assert!(!m.shared_pool().contains(0));
        assert!(!m.shared_pool().contains(1));
    }

    #[test]
    fn allocate_none_policy_returns_empty() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::None, CpuSet::new()).unwrap();
        let s = m.allocate("p", "c", 4, true).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn allocate_static_policy_assigns_exclusively() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let s = m.allocate("p", "c", 4, true).unwrap();
        assert_eq!(s.size(), 4);
        // Shared pool shrinks.
        assert_eq!(m.shared_pool().size(), 12);
    }

    #[test]
    fn allocate_skipped_for_non_guaranteed_pods() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let s = m.allocate("p", "c", 4, false).unwrap();
        assert!(s.is_empty());
    }

    #[test]
    fn allocate_zero_request_returns_empty() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        assert!(m.allocate("p", "c", 0, true).unwrap().is_empty());
    }

    #[test]
    fn allocate_insufficient_returns_err() {
        let t = CpuTopology::make_homogeneous(1, 2, 1);
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let err = m.allocate("p", "c", 5, true).unwrap_err();
        assert!(matches!(err, CpuError::Insufficient { .. }));
    }

    #[test]
    fn allocate_idempotent_for_same_request() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let a = m.allocate("p", "c", 4, true).unwrap();
        let b = m.allocate("p", "c", 4, true).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn allocate_conflict_when_request_changes() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        m.allocate("p", "c", 4, true).unwrap();
        let err = m.allocate("p", "c", 8, true).unwrap_err();
        assert!(matches!(err, CpuError::Conflict(_)));
    }

    #[test]
    fn deallocate_returns_cpus_to_shared_pool() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        m.allocate("p", "c", 4, true).unwrap();
        m.deallocate("p", "c");
        assert_eq!(m.shared_pool().size(), 16);
        assert!(m.assignment_for("p", "c").is_none());
    }

    #[test]
    fn deallocate_unknown_is_noop() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        m.deallocate("ghost", "ghost");
    }

    #[test]
    fn allocate_full_socket_when_request_equals_socket_size() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t.clone(), CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let s = m.allocate("p", "c", 8, true).unwrap();
        // Should match an entire socket.
        let s0 = t.cpus_in_socket(0);
        let s1 = t.cpus_in_socket(1);
        assert!(s == s0 || s == s1);
    }

    #[test]
    fn allocate_full_core_when_request_equals_core_size() {
        let t = CpuTopology::make_homogeneous(1, 4, 2); // 8 cpus, 1 socket of size 8
        let mut m = CpuManager::new(t.clone(), CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let s = m.allocate("p", "c", 2, true).unwrap();
        // Should pick both threads of one core.
        assert_eq!(s.size(), 2);
        let core_id = t.cpus[s.iter().next().unwrap()].core_id;
        for c in s.iter() {
            assert_eq!(t.cpus[c].core_id, core_id);
        }
    }

    #[test]
    fn allocate_subsequent_requests_use_separate_cpus() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        let a = m.allocate("p1", "c", 4, true).unwrap();
        let b = m.allocate("p2", "c", 4, true).unwrap();
        assert!(a.intersection(&b).is_empty());
    }

    #[test]
    fn allocated_cpus_aggregates_assignments() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        m.allocate("p1", "c", 2, true).unwrap();
        m.allocate("p2", "c", 4, true).unwrap();
        assert_eq!(m.allocated_cpus().size(), 6);
    }

    #[test]
    fn full_pcpus_only_rejects_odd_request_on_smt() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new())
            .unwrap()
            .with_options(CpuManagerOptions {
                full_pcpus_only: true,
                ..Default::default()
            });
        let err = m.allocate("p", "c", 1, true).unwrap_err();
        assert!(matches!(err, CpuError::Invalid(_)));
    }

    #[test]
    fn full_pcpus_only_accepts_even_request_on_smt() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new())
            .unwrap()
            .with_options(CpuManagerOptions {
                full_pcpus_only: true,
                ..Default::default()
            });
        m.allocate("p", "c", 4, true).unwrap();
    }

    #[test]
    fn pick_cpus_zero_returns_empty() {
        let t = topo_2s_4c_2t();
        let m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        assert!(m.pick_cpus(0).unwrap().is_empty());
    }

    #[test]
    fn pick_cpus_insufficient_when_pool_too_small() {
        let t = CpuTopology::make_homogeneous(1, 1, 1);
        let m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        assert!(m.pick_cpus(2).is_err());
    }

    #[test]
    fn assignments_count_grows_and_shrinks() {
        let t = topo_2s_4c_2t();
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        m.allocate("p1", "c", 2, true).unwrap();
        m.allocate("p2", "c", 2, true).unwrap();
        assert_eq!(m.assignments_count(), 2);
        m.deallocate("p1", "c");
        assert_eq!(m.assignments_count(), 1);
    }

    #[test]
    fn cpuset_to_vec_sorted() {
        let s = CpuSet::from_iter(vec![3, 1, 2]);
        assert_eq!(s.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn assignment_for_unknown_returns_none() {
        let t = topo_2s_4c_2t();
        let m = CpuManager::new(t, CpuManagerPolicy::Static, CpuSet::new()).unwrap();
        assert!(m.assignment_for("p", "c").is_none());
    }

    #[test]
    fn allocate_uses_only_unreserved_cpus() {
        let t = topo_2s_4c_2t();
        let reserved = CpuSet::from_iter(vec![0, 1]);
        let mut m = CpuManager::new(t, CpuManagerPolicy::Static, reserved.clone()).unwrap();
        let s = m.allocate("p", "c", 14, true).unwrap();
        assert_eq!(s.size(), 14);
        assert!(s.intersection(&reserved).is_empty());
    }
}
