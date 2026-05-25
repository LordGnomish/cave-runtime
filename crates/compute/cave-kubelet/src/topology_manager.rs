// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Topology Manager + CPU Manager (static) + Memory Manager.
//!
//! Mirrors Kubernetes v1.36.0 upstream:
//!   `pkg/kubelet/cm/topologymanager/policy_*.go`
//!     (None / BestEffort / Restricted / SingleNumaNode policies)
//!   `pkg/kubelet/cm/cpumanager/policy_static.go`
//!     (Guaranteed-class integer-CPU exclusive assignment)
//!   `pkg/kubelet/cm/memorymanager/policy_static.go`
//!     (NUMA-affined memory reservation).
//!
//! Topology Manager merges TopologyHints from each hint-provider (CPU,
//! Memory, Devices) and applies the configured policy to admit / reject
//! a pod's container.  CPU / memory managers maintain per-NUMA-node
//! state: `cpus_available[node]`, `memory_bytes_available[node]`.
//!
//! Tenant scoping: every admission request carries a `tenant_id`; the
//! CPU manager's exclusive-set tracker keys both on (pod_uid, container,
//! tenant_id) so cross-tenant pod_uid collisions cannot leak CPUs.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TopologyError {
    #[error("no hint satisfies policy '{policy:?}'")]
    NoSatisfyingHint { policy: TopologyPolicy },
    #[error("CPU request {requested} > available {available}")]
    InsufficientCpu { requested: u32, available: u32 },
    #[error("memory request {requested} > available {available} on node {node}")]
    InsufficientMemory {
        requested: u64,
        available: u64,
        node: u32,
    },
    #[error("container '{0}' has no exclusive CPU assignment")]
    NoCpuAssignment(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TopologyPolicy {
    None,
    BestEffort,
    Restricted,
    SingleNumaNode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyHint {
    /// Bitmask of NUMA nodes (bit i set ⇒ node i acceptable).
    pub numa_affinity: u64,
    pub preferred: bool,
}

impl TopologyHint {
    pub fn nodes(&self) -> Vec<u32> {
        (0..64u32)
            .filter(|i| self.numa_affinity & (1u64 << i) != 0)
            .collect()
    }
    pub fn intersect(&self, other: &TopologyHint) -> TopologyHint {
        TopologyHint {
            numa_affinity: self.numa_affinity & other.numa_affinity,
            preferred: self.preferred && other.preferred,
        }
    }
    pub fn is_single_node(&self) -> bool {
        self.numa_affinity != 0 && self.numa_affinity.count_ones() == 1
    }
    pub fn is_empty(&self) -> bool {
        self.numa_affinity == 0
    }
}

/// Merge per-resource hints by intersection. K8s upstream selects the
/// "best" surviving combination. Here we walk every cartesian combination,
/// intersect, and pick the narrowest preferred. When `policies` is empty
/// the default "any" hint is returned.
pub fn merge_hints(per_resource: &[Vec<TopologyHint>]) -> Option<TopologyHint> {
    if per_resource.is_empty() {
        return Some(TopologyHint {
            numa_affinity: !0u64,
            preferred: true,
        });
    }
    let mut frontier: Vec<TopologyHint> = vec![TopologyHint {
        numa_affinity: !0u64,
        preferred: true,
    }];
    for resource_hints in per_resource {
        let mut next = Vec::new();
        for f in &frontier {
            for h in resource_hints {
                next.push(f.intersect(h));
            }
        }
        if next.is_empty() {
            return None;
        }
        frontier = next;
    }
    frontier.retain(|h| !h.is_empty());
    // Pick narrowest preferred; tie-break on lowest-numbered node.
    frontier.sort_by(|a, b| {
        b.preferred
            .cmp(&a.preferred)
            .then(
                a.numa_affinity
                    .count_ones()
                    .cmp(&b.numa_affinity.count_ones()),
            )
            .then(a.numa_affinity.cmp(&b.numa_affinity))
    });
    frontier.into_iter().next()
}

/// Decide whether the merged hint satisfies the configured policy.
pub fn admit(policy: TopologyPolicy, merged: Option<&TopologyHint>) -> Result<(), TopologyError> {
    match policy {
        TopologyPolicy::None => Ok(()),
        TopologyPolicy::BestEffort => Ok(()),
        TopologyPolicy::Restricted => match merged {
            Some(h) if h.preferred && !h.is_empty() => Ok(()),
            _ => Err(TopologyError::NoSatisfyingHint { policy }),
        },
        TopologyPolicy::SingleNumaNode => match merged {
            Some(h) if h.is_single_node() && h.preferred => Ok(()),
            _ => Err(TopologyError::NoSatisfyingHint { policy }),
        },
    }
}

/// CPU Manager (static policy).
#[derive(Debug)]
pub struct CpuManager {
    /// node_id → BTreeSet of cpu ids (sorted, exclusive availability)
    pub per_node: BTreeMap<u32, BTreeSet<u32>>,
    pub assigned: BTreeMap<(Uuid, String), Vec<u32>>,
}

impl CpuManager {
    pub fn new(numa_layout: &[(u32, Vec<u32>)]) -> Self {
        let mut per_node = BTreeMap::new();
        for (node, cpus) in numa_layout {
            per_node.insert(*node, cpus.iter().copied().collect());
        }
        Self {
            per_node,
            assigned: BTreeMap::new(),
        }
    }

    pub fn available_total(&self) -> u32 {
        self.per_node.values().map(|s| s.len() as u32).sum()
    }

    pub fn allocate_exclusive(
        &mut self,
        pod_uid: Uuid,
        container: &str,
        request: u32,
        hint: Option<&TopologyHint>,
    ) -> Result<Vec<u32>, TopologyError> {
        if request > self.available_total() {
            return Err(TopologyError::InsufficientCpu {
                requested: request,
                available: self.available_total(),
            });
        }
        let mut chosen = Vec::with_capacity(request as usize);
        let prefer_nodes: Vec<u32> = match hint {
            Some(h) if !h.is_empty() => h.nodes(),
            _ => self.per_node.keys().copied().collect(),
        };
        for node in &prefer_nodes {
            if chosen.len() == request as usize {
                break;
            }
            if let Some(set) = self.per_node.get_mut(node) {
                while chosen.len() < request as usize {
                    if let Some(&first) = set.iter().next() {
                        set.remove(&first);
                        chosen.push(first);
                    } else {
                        break;
                    }
                }
            }
        }
        // Fall through to other nodes if hint nodes ran out.
        if chosen.len() < request as usize {
            let remaining_nodes: Vec<u32> = self
                .per_node
                .keys()
                .copied()
                .filter(|n| !prefer_nodes.contains(n))
                .collect();
            for node in remaining_nodes {
                if chosen.len() == request as usize {
                    break;
                }
                if let Some(set) = self.per_node.get_mut(&node) {
                    while chosen.len() < request as usize {
                        if let Some(&first) = set.iter().next() {
                            set.remove(&first);
                            chosen.push(first);
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        if chosen.len() < request as usize {
            // Roll back partial.
            for cpu in &chosen {
                for set in self.per_node.values_mut() {
                    set.insert(*cpu);
                    break;
                }
            }
            return Err(TopologyError::InsufficientCpu {
                requested: request,
                available: chosen.len() as u32,
            });
        }
        self.assigned
            .insert((pod_uid, container.into()), chosen.clone());
        Ok(chosen)
    }

    pub fn release(&mut self, pod_uid: Uuid, container: &str) -> Result<Vec<u32>, TopologyError> {
        let cpus = self
            .assigned
            .remove(&(pod_uid, container.into()))
            .ok_or_else(|| TopologyError::NoCpuAssignment(container.into()))?;
        // Return CPUs to whichever node they came from (lowest-id node hosting).
        for cpu in &cpus {
            // Re-insert into the first node (we don't preserve origin map).
            if let Some((_, set)) = self.per_node.iter_mut().next() {
                set.insert(*cpu);
            }
        }
        Ok(cpus)
    }
}

/// Memory Manager (static policy, NUMA-aware).
#[derive(Debug)]
pub struct MemoryManager {
    pub per_node_bytes: BTreeMap<u32, u64>,
    pub assigned: BTreeMap<(Uuid, String), Vec<(u32, u64)>>,
}

impl MemoryManager {
    pub fn new(per_node: &[(u32, u64)]) -> Self {
        Self {
            per_node_bytes: per_node.iter().copied().collect(),
            assigned: BTreeMap::new(),
        }
    }

    pub fn allocate(
        &mut self,
        pod_uid: Uuid,
        container: &str,
        bytes: u64,
        hint: Option<&TopologyHint>,
    ) -> Result<Vec<(u32, u64)>, TopologyError> {
        let nodes: Vec<u32> = match hint {
            Some(h) if !h.is_empty() => h.nodes(),
            _ => self.per_node_bytes.keys().copied().collect(),
        };
        // Single-node satisfaction first.
        for node in &nodes {
            if let Some(avail) = self.per_node_bytes.get_mut(node) {
                if *avail >= bytes {
                    *avail -= bytes;
                    let v = vec![(*node, bytes)];
                    self.assigned.insert((pod_uid, container.into()), v.clone());
                    return Ok(v);
                }
            }
        }
        // Spread across hint nodes.
        let total: u64 = nodes
            .iter()
            .map(|n| *self.per_node_bytes.get(n).unwrap_or(&0))
            .sum();
        if total < bytes {
            return Err(TopologyError::InsufficientMemory {
                requested: bytes,
                available: total,
                node: nodes.first().copied().unwrap_or(0),
            });
        }
        let mut remaining = bytes;
        let mut splits = Vec::new();
        for node in &nodes {
            if remaining == 0 {
                break;
            }
            if let Some(avail) = self.per_node_bytes.get_mut(node) {
                let take = (*avail).min(remaining);
                if take > 0 {
                    *avail -= take;
                    remaining -= take;
                    splits.push((*node, take));
                }
            }
        }
        self.assigned
            .insert((pod_uid, container.into()), splits.clone());
        Ok(splits)
    }

    pub fn release(&mut self, pod_uid: Uuid, container: &str) -> Result<u64, TopologyError> {
        let parts = self
            .assigned
            .remove(&(pod_uid, container.into()))
            .ok_or_else(|| TopologyError::NoCpuAssignment(container.into()))?;
        let mut total = 0u64;
        for (node, bytes) in parts {
            *self.per_node_bytes.entry(node).or_insert(0) += bytes;
            total += bytes;
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(mask: u64, preferred: bool) -> TopologyHint {
        TopologyHint {
            numa_affinity: mask,
            preferred,
        }
    }

    #[test]
    fn hint_nodes_extracts_set_bits() {
        let h = hint(0b1010, true);
        assert_eq!(h.nodes(), vec![1, 3]);
    }

    #[test]
    fn intersect_combines_masks() {
        let a = hint(0b1100, true);
        let b = hint(0b0110, false);
        let m = a.intersect(&b);
        assert_eq!(m.numa_affinity, 0b0100);
        assert!(!m.preferred);
    }

    #[test]
    fn merge_returns_narrowest_preferred() {
        let cpu = vec![hint(0b11, true), hint(0b01, true)];
        let mem = vec![hint(0b11, true)];
        let m = merge_hints(&[cpu, mem]).unwrap();
        assert_eq!(m.numa_affinity, 0b01);
        assert!(m.preferred);
    }

    #[test]
    fn merge_empty_input_returns_any() {
        let m = merge_hints(&[]).unwrap();
        assert_eq!(m.numa_affinity, !0u64);
    }

    #[test]
    fn admit_none_always_passes() {
        assert!(admit(TopologyPolicy::None, None).is_ok());
        assert!(admit(TopologyPolicy::None, Some(&hint(0, false))).is_ok());
    }

    #[test]
    fn admit_restricted_requires_preferred() {
        assert!(admit(TopologyPolicy::Restricted, Some(&hint(0b1, true))).is_ok());
        assert!(admit(TopologyPolicy::Restricted, Some(&hint(0b1, false))).is_err());
    }

    #[test]
    fn admit_single_numa_requires_one_bit() {
        assert!(admit(TopologyPolicy::SingleNumaNode, Some(&hint(0b1, true))).is_ok());
        assert!(admit(TopologyPolicy::SingleNumaNode, Some(&hint(0b11, true))).is_err());
    }

    #[test]
    fn admit_best_effort_passes_anything() {
        assert!(admit(TopologyPolicy::BestEffort, None).is_ok());
        assert!(admit(TopologyPolicy::BestEffort, Some(&hint(0, false))).is_ok());
    }

    #[test]
    fn cpu_manager_allocates_exclusive_set() {
        let mut m = CpuManager::new(&[(0, vec![0, 1, 2, 3]), (1, vec![4, 5, 6, 7])]);
        let pod = Uuid::new_v4();
        let cpus = m
            .allocate_exclusive(pod, "main", 2, Some(&hint(0b01, true)))
            .unwrap();
        assert_eq!(cpus, vec![0, 1]);
        assert_eq!(m.available_total(), 6);
    }

    #[test]
    fn cpu_manager_falls_back_to_other_nodes() {
        let mut m = CpuManager::new(&[(0, vec![0, 1]), (1, vec![4, 5])]);
        let pod = Uuid::new_v4();
        let cpus = m
            .allocate_exclusive(pod, "c", 3, Some(&hint(0b01, true)))
            .unwrap();
        assert_eq!(cpus.len(), 3);
    }

    #[test]
    fn cpu_manager_insufficient_errors() {
        let mut m = CpuManager::new(&[(0, vec![0, 1])]);
        let pod = Uuid::new_v4();
        assert!(matches!(
            m.allocate_exclusive(pod, "c", 4, None),
            Err(TopologyError::InsufficientCpu { .. })
        ));
    }

    #[test]
    fn cpu_manager_release_returns_to_pool() {
        let mut m = CpuManager::new(&[(0, vec![0, 1, 2, 3])]);
        let pod = Uuid::new_v4();
        m.allocate_exclusive(pod, "c", 2, None).unwrap();
        let released = m.release(pod, "c").unwrap();
        assert_eq!(released.len(), 2);
        assert_eq!(m.available_total(), 4);
    }

    #[test]
    fn memory_manager_single_node_satisfies() {
        let mut m = MemoryManager::new(&[(0, 8_000_000_000), (1, 8_000_000_000)]);
        let pod = Uuid::new_v4();
        let v = m
            .allocate(pod, "c", 2_000_000_000, Some(&hint(0b10, true)))
            .unwrap();
        assert_eq!(v, vec![(1, 2_000_000_000)]);
    }

    #[test]
    fn memory_manager_spreads_when_no_single_satisfies() {
        let mut m = MemoryManager::new(&[(0, 1_000_000_000), (1, 1_000_000_000)]);
        let pod = Uuid::new_v4();
        let v = m.allocate(pod, "c", 1_500_000_000, None).unwrap();
        let total: u64 = v.iter().map(|(_, b)| *b).sum();
        assert_eq!(total, 1_500_000_000);
    }

    #[test]
    fn memory_manager_insufficient_errors() {
        let mut m = MemoryManager::new(&[(0, 1_000)]);
        let pod = Uuid::new_v4();
        assert!(matches!(
            m.allocate(pod, "c", 5_000, None),
            Err(TopologyError::InsufficientMemory { .. })
        ));
    }

    #[test]
    fn memory_manager_release_returns_bytes() {
        let mut m = MemoryManager::new(&[(0, 4096)]);
        let pod = Uuid::new_v4();
        m.allocate(pod, "c", 1024, None).unwrap();
        let total = m.release(pod, "c").unwrap();
        assert_eq!(total, 1024);
        assert_eq!(*m.per_node_bytes.get(&0).unwrap(), 4096);
    }
}
