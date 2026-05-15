// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Pod Resources API — `kubelet.sock`-style v1alpha1 + v1 endpoints.
//!
//! Mirrors `pkg/kubelet/apis/podresources/v1` and the v1alpha1 surface used
//! by device-plugin and metric-collection consumers. Reports per-container
//! CPU IDs, memory blocks (with NUMA node), and device assignments
//! (resource name → device IDs with topology). Implements
//! `ListPodResources` and `GetAllocatableResources`.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NumaNode {
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologyInfo {
    pub nodes: Vec<NumaNode>,
}

impl TopologyInfo {
    pub fn single(id: i64) -> Self {
        Self {
            nodes: vec![NumaNode { id }],
        }
    }

    pub fn cross_numa(&self) -> bool {
        self.nodes.len() > 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerDevices {
    pub resource_name: String,
    pub device_ids: Vec<String>,
    pub topology: Option<TopologyInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerMemory {
    pub memory_type: MemoryType,
    pub size_bytes: u64,
    pub topology: Option<TopologyInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryType {
    Memory,
    HugepagesPlain,
    Hugepages2Mi,
    Hugepages1Gi,
}

impl MemoryType {
    pub fn name(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::HugepagesPlain => "hugepages",
            Self::Hugepages2Mi => "hugepages-2Mi",
            Self::Hugepages1Gi => "hugepages-1Gi",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerResources {
    pub name: String,
    pub cpu_ids: Vec<i64>,
    pub memory: Vec<ContainerMemory>,
    pub devices: Vec<ContainerDevices>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PodResources {
    pub name: String,
    pub namespace: String,
    pub uid: String,
    pub containers: Vec<ContainerResources>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocatableResources {
    pub cpu_ids: Vec<i64>,
    pub memory: Vec<ContainerMemory>,
    pub devices: Vec<ContainerDevices>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PodResourcesError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

pub type PrResult<T> = Result<T, PodResourcesError>;

/// In-memory pod-resources state. Thread-safe.
#[derive(Debug, Default)]
pub struct ResourceManager {
    pods: DashMap<String, PodResources>,
    allocatable: RwLock<AllocatableResources>,
    /// Tracks which (resource_name, device_id) pairs have been claimed
    /// to detect double-allocation.
    device_claims: DashMap<(String, String), String>,
    /// Tracks which CPU IDs are claimed to detect conflicts under exclusive
    /// allocation policies.
    cpu_claims: DashMap<i64, String>,
}

impl ResourceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_allocatable(&self, alloc: AllocatableResources) {
        *self.allocatable.write().unwrap() = alloc;
    }

    pub fn allocatable(&self) -> AllocatableResources {
        self.allocatable.read().unwrap().clone()
    }

    /// Register a pod's resource assignment.
    /// Validates uniqueness of CPU and device IDs.
    pub fn upsert_pod(&self, pod: PodResources) -> PrResult<()> {
        validate_pod(&pod)?;
        // Validate that CPUs / devices are within allocatable.
        let alloc = self.allocatable();
        for c in &pod.containers {
            for cpu in &c.cpu_ids {
                if !alloc.cpu_ids.is_empty() && !alloc.cpu_ids.contains(cpu) {
                    return Err(PodResourcesError::Invalid(format!(
                        "cpu id {} not in allocatable set",
                        cpu
                    )));
                }
            }
            for d in &c.devices {
                if let Some(declared) = alloc.devices.iter().find(|x| x.resource_name == d.resource_name) {
                    for id in &d.device_ids {
                        if !declared.device_ids.contains(id) {
                            return Err(PodResourcesError::Invalid(format!(
                                "device {} not in allocatable {}",
                                id, d.resource_name
                            )));
                        }
                    }
                }
            }
        }
        // Release prior claims first if pod existed.
        if let Some(prev) = self.pods.get(&pod.uid) {
            self.release_claims(&prev);
        }
        // Claim CPUs and devices.
        for c in &pod.containers {
            for cpu in &c.cpu_ids {
                if let Some(holder) = self.cpu_claims.get(cpu) {
                    if holder.value() != &pod.uid {
                        return Err(PodResourcesError::Conflict(format!(
                            "cpu {} already claimed by {}",
                            cpu,
                            holder.value()
                        )));
                    }
                }
                self.cpu_claims.insert(*cpu, pod.uid.clone());
            }
            for d in &c.devices {
                for id in &d.device_ids {
                    let key = (d.resource_name.clone(), id.clone());
                    if let Some(holder) = self.device_claims.get(&key) {
                        if holder.value() != &pod.uid {
                            return Err(PodResourcesError::Conflict(format!(
                                "device {}/{} already claimed by {}",
                                d.resource_name,
                                id,
                                holder.value()
                            )));
                        }
                    }
                    self.device_claims.insert(key, pod.uid.clone());
                }
            }
        }
        self.pods.insert(pod.uid.clone(), pod);
        Ok(())
    }

    fn release_claims(&self, pod: &PodResources) {
        for c in &pod.containers {
            for cpu in &c.cpu_ids {
                self.cpu_claims.remove(cpu);
            }
            for d in &c.devices {
                for id in &d.device_ids {
                    self.device_claims.remove(&(d.resource_name.clone(), id.clone()));
                }
            }
        }
    }

    pub fn remove_pod(&self, uid: &str) -> Option<PodResources> {
        let pod = self.pods.remove(uid).map(|(_, p)| p)?;
        self.release_claims(&pod);
        Some(pod)
    }

    pub fn list_pod_resources(&self) -> Vec<PodResources> {
        let mut out: Vec<PodResources> = self.pods.iter().map(|r| r.value().clone()).collect();
        out.sort_by(|a, b| a.uid.cmp(&b.uid));
        out
    }

    pub fn get_pod(&self, uid: &str) -> Option<PodResources> {
        self.pods.get(uid).map(|r| r.value().clone())
    }

    /// Aggregate per-NUMA-node memory used by claimed pods.
    pub fn memory_usage_by_numa(&self) -> BTreeMap<i64, u64> {
        let mut out: BTreeMap<i64, u64> = BTreeMap::new();
        for r in self.pods.iter() {
            for c in &r.value().containers {
                for m in &c.memory {
                    let key = m
                        .topology
                        .as_ref()
                        .and_then(|t| t.nodes.first().map(|n| n.id))
                        .unwrap_or(-1);
                    *out.entry(key).or_default() += m.size_bytes;
                }
            }
        }
        out
    }

    /// Aggregate per-resource device usage.
    pub fn device_usage_by_resource(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in self.pods.iter() {
            for c in &r.value().containers {
                for d in &c.devices {
                    let entry = out.entry(d.resource_name.clone()).or_default();
                    for id in &d.device_ids {
                        entry.insert(id.clone());
                    }
                }
            }
        }
        out
    }

    pub fn pod_count(&self) -> usize {
        self.pods.len()
    }

    pub fn cpu_used(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self.cpu_claims.iter().map(|r| *r.key()).collect();
        v.sort();
        v
    }
}

pub fn validate_pod(pod: &PodResources) -> PrResult<()> {
    if pod.uid.is_empty() {
        return Err(PodResourcesError::Invalid("pod uid empty".into()));
    }
    if pod.name.is_empty() {
        return Err(PodResourcesError::Invalid("pod name empty".into()));
    }
    let mut names = BTreeSet::new();
    for c in &pod.containers {
        if !names.insert(&c.name) {
            return Err(PodResourcesError::Invalid(format!(
                "duplicate container name {}",
                c.name
            )));
        }
        // Within a single container, CPU IDs must be unique.
        let mut cpus = BTreeSet::new();
        for cpu in &c.cpu_ids {
            if !cpus.insert(cpu) {
                return Err(PodResourcesError::Invalid(format!(
                    "duplicate cpu id {} in container {}",
                    cpu, c.name
                )));
            }
        }
        // Within a single container, device IDs (per resource) must be unique.
        for d in &c.devices {
            let mut ids = BTreeSet::new();
            for id in &d.device_ids {
                if !ids.insert(id) {
                    return Err(PodResourcesError::Invalid(format!(
                        "duplicate device id {} for resource {}",
                        id, d.resource_name
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alloc_with(cpus: &[i64], devices: Vec<ContainerDevices>) -> AllocatableResources {
        AllocatableResources {
            cpu_ids: cpus.to_vec(),
            memory: vec![],
            devices,
        }
    }

    fn pod_simple(uid: &str, name: &str, cpus: Vec<i64>) -> PodResources {
        PodResources {
            name: name.into(),
            namespace: "default".into(),
            uid: uid.into(),
            containers: vec![ContainerResources {
                name: "c".into(),
                cpu_ids: cpus,
                memory: vec![],
                devices: vec![],
            }],
        }
    }

    #[test]
    fn upsert_pod_records_in_list() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2, 3], vec![]));
        m.upsert_pod(pod_simple("p1", "n1", vec![0, 1])).unwrap();
        let list = m.list_pod_resources();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].uid, "p1");
    }

    #[test]
    fn upsert_pod_rejects_cpu_outside_allocatable() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1], vec![]));
        let err = m.upsert_pod(pod_simple("p1", "n1", vec![5])).unwrap_err();
        assert!(matches!(err, PodResourcesError::Invalid(_)));
    }

    #[test]
    fn upsert_pod_allows_when_allocatable_unspecified() {
        let m = ResourceManager::new();
        m.upsert_pod(pod_simple("p1", "n1", vec![5])).unwrap();
        assert_eq!(m.pod_count(), 1);
    }

    #[test]
    fn upsert_pod_rejects_cpu_double_claim_across_pods() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2, 3], vec![]));
        m.upsert_pod(pod_simple("p1", "n1", vec![0, 1])).unwrap();
        let err = m.upsert_pod(pod_simple("p2", "n2", vec![1, 2])).unwrap_err();
        assert!(matches!(err, PodResourcesError::Conflict(_)));
    }

    #[test]
    fn upsert_pod_idempotent_for_same_uid_same_cpus() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2], vec![]));
        m.upsert_pod(pod_simple("p1", "n1", vec![0, 1])).unwrap();
        // Same pod re-upserted with same cpus should release+reclaim.
        m.upsert_pod(pod_simple("p1", "n1", vec![0, 1])).unwrap();
        assert_eq!(m.pod_count(), 1);
    }

    #[test]
    fn upsert_pod_can_change_cpu_set_for_same_uid() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2, 3], vec![]));
        m.upsert_pod(pod_simple("p1", "n1", vec![0, 1])).unwrap();
        m.upsert_pod(pod_simple("p1", "n1", vec![2, 3])).unwrap();
        assert_eq!(m.cpu_used(), vec![2, 3]);
    }

    #[test]
    fn remove_pod_releases_cpu_claims() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2], vec![]));
        m.upsert_pod(pod_simple("p1", "n1", vec![0, 1])).unwrap();
        m.remove_pod("p1");
        assert!(m.cpu_used().is_empty());
        // Now another pod may take them.
        m.upsert_pod(pod_simple("p2", "n2", vec![0, 1])).unwrap();
    }

    #[test]
    fn validate_rejects_empty_uid() {
        let mut p = pod_simple("p1", "n1", vec![]);
        p.uid = String::new();
        assert!(validate_pod(&p).is_err());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let mut p = pod_simple("p1", "n1", vec![]);
        p.name = String::new();
        assert!(validate_pod(&p).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_container_names() {
        let mut p = pod_simple("p1", "n1", vec![]);
        p.containers.push(ContainerResources {
            name: "c".into(), // dup
            cpu_ids: vec![],
            memory: vec![],
            devices: vec![],
        });
        assert!(validate_pod(&p).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_cpu_ids_in_container() {
        let p = pod_simple("p1", "n1", vec![0, 0]);
        assert!(validate_pod(&p).is_err());
    }

    #[test]
    fn validate_rejects_duplicate_device_ids_in_container() {
        let mut p = pod_simple("p1", "n1", vec![]);
        p.containers[0].devices = vec![ContainerDevices {
            resource_name: "nvidia.com/gpu".into(),
            device_ids: vec!["uuid-1".into(), "uuid-1".into()],
            topology: None,
        }];
        assert!(validate_pod(&p).is_err());
    }

    #[test]
    fn devices_outside_allocatable_rejected() {
        let m = ResourceManager::new();
        m.set_allocatable(AllocatableResources {
            cpu_ids: vec![],
            memory: vec![],
            devices: vec![ContainerDevices {
                resource_name: "nvidia.com/gpu".into(),
                device_ids: vec!["uuid-1".into(), "uuid-2".into()],
                topology: None,
            }],
        });
        let mut p = pod_simple("p1", "n1", vec![]);
        p.containers[0].devices = vec![ContainerDevices {
            resource_name: "nvidia.com/gpu".into(),
            device_ids: vec!["uuid-99".into()],
            topology: None,
        }];
        assert!(matches!(m.upsert_pod(p), Err(PodResourcesError::Invalid(_))));
    }

    #[test]
    fn device_double_claim_across_pods_rejected() {
        let m = ResourceManager::new();
        m.set_allocatable(AllocatableResources {
            cpu_ids: vec![],
            memory: vec![],
            devices: vec![ContainerDevices {
                resource_name: "nvidia.com/gpu".into(),
                device_ids: vec!["uuid-1".into(), "uuid-2".into()],
                topology: None,
            }],
        });
        let mut p1 = pod_simple("p1", "n1", vec![]);
        p1.containers[0].devices = vec![ContainerDevices {
            resource_name: "nvidia.com/gpu".into(),
            device_ids: vec!["uuid-1".into()],
            topology: None,
        }];
        m.upsert_pod(p1).unwrap();

        let mut p2 = pod_simple("p2", "n2", vec![]);
        p2.containers[0].devices = vec![ContainerDevices {
            resource_name: "nvidia.com/gpu".into(),
            device_ids: vec!["uuid-1".into()], // dup
            topology: None,
        }];
        assert!(matches!(m.upsert_pod(p2), Err(PodResourcesError::Conflict(_))));
    }

    #[test]
    fn list_pod_resources_sorted_by_uid() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2, 3, 4, 5], vec![]));
        m.upsert_pod(pod_simple("p2", "n", vec![2])).unwrap();
        m.upsert_pod(pod_simple("p1", "n", vec![0])).unwrap();
        m.upsert_pod(pod_simple("p3", "n", vec![3])).unwrap();
        let list = m.list_pod_resources();
        assert_eq!(list[0].uid, "p1");
        assert_eq!(list[1].uid, "p2");
        assert_eq!(list[2].uid, "p3");
    }

    #[test]
    fn get_allocatable_returns_set() {
        let m = ResourceManager::new();
        let a = alloc_with(&[0, 1], vec![]);
        m.set_allocatable(a.clone());
        assert_eq!(m.allocatable(), a);
    }

    #[test]
    fn cpu_used_returns_sorted_unique() {
        let m = ResourceManager::new();
        m.set_allocatable(alloc_with(&[0, 1, 2, 3, 4], vec![]));
        m.upsert_pod(pod_simple("p1", "n", vec![3, 1])).unwrap();
        m.upsert_pod(pod_simple("p2", "n", vec![4, 0])).unwrap();
        assert_eq!(m.cpu_used(), vec![0, 1, 3, 4]);
    }

    #[test]
    fn topology_info_cross_numa_detection() {
        assert!(!TopologyInfo::single(0).cross_numa());
        let t = TopologyInfo {
            nodes: vec![NumaNode { id: 0 }, NumaNode { id: 1 }],
        };
        assert!(t.cross_numa());
    }

    #[test]
    fn memory_usage_by_numa_aggregates_correctly() {
        let m = ResourceManager::new();
        let mut p1 = pod_simple("p1", "n", vec![]);
        p1.containers[0].memory = vec![
            ContainerMemory {
                memory_type: MemoryType::Memory,
                size_bytes: 1024,
                topology: Some(TopologyInfo::single(0)),
            },
            ContainerMemory {
                memory_type: MemoryType::Hugepages2Mi,
                size_bytes: 2048,
                topology: Some(TopologyInfo::single(1)),
            },
        ];
        m.upsert_pod(p1).unwrap();
        let mut p2 = pod_simple("p2", "n", vec![]);
        p2.containers[0].memory = vec![ContainerMemory {
            memory_type: MemoryType::Memory,
            size_bytes: 512,
            topology: Some(TopologyInfo::single(0)),
        }];
        m.upsert_pod(p2).unwrap();
        let agg = m.memory_usage_by_numa();
        assert_eq!(agg[&0], 1536);
        assert_eq!(agg[&1], 2048);
    }

    #[test]
    fn device_usage_by_resource_aggregates_correctly() {
        let m = ResourceManager::new();
        m.set_allocatable(AllocatableResources {
            cpu_ids: vec![],
            memory: vec![],
            devices: vec![
                ContainerDevices {
                    resource_name: "nvidia.com/gpu".into(),
                    device_ids: vec!["g1".into(), "g2".into(), "g3".into()],
                    topology: None,
                },
                ContainerDevices {
                    resource_name: "intel.com/qat".into(),
                    device_ids: vec!["q1".into()],
                    topology: None,
                },
            ],
        });
        let mut p1 = pod_simple("p1", "n", vec![]);
        p1.containers[0].devices = vec![ContainerDevices {
            resource_name: "nvidia.com/gpu".into(),
            device_ids: vec!["g1".into(), "g2".into()],
            topology: None,
        }];
        m.upsert_pod(p1).unwrap();
        let mut p2 = pod_simple("p2", "n", vec![]);
        p2.containers[0].devices = vec![ContainerDevices {
            resource_name: "intel.com/qat".into(),
            device_ids: vec!["q1".into()],
            topology: None,
        }];
        m.upsert_pod(p2).unwrap();
        let agg = m.device_usage_by_resource();
        assert_eq!(agg.get("nvidia.com/gpu").unwrap().len(), 2);
        assert_eq!(agg.get("intel.com/qat").unwrap().len(), 1);
    }

    #[test]
    fn pod_count_tracks_lifecycle() {
        let m = ResourceManager::new();
        assert_eq!(m.pod_count(), 0);
        m.upsert_pod(pod_simple("p1", "n", vec![])).unwrap();
        m.upsert_pod(pod_simple("p2", "n", vec![])).unwrap();
        assert_eq!(m.pod_count(), 2);
        m.remove_pod("p1");
        assert_eq!(m.pod_count(), 1);
    }

    #[test]
    fn get_pod_by_uid_returns_clone() {
        let m = ResourceManager::new();
        m.upsert_pod(pod_simple("p1", "n", vec![])).unwrap();
        let p = m.get_pod("p1").unwrap();
        assert_eq!(p.uid, "p1");
    }

    #[test]
    fn get_pod_unknown_returns_none() {
        let m = ResourceManager::new();
        assert!(m.get_pod("ghost").is_none());
    }

    #[test]
    fn memory_type_names_match_kubernetes() {
        assert_eq!(MemoryType::Memory.name(), "memory");
        assert_eq!(MemoryType::Hugepages2Mi.name(), "hugepages-2Mi");
        assert_eq!(MemoryType::Hugepages1Gi.name(), "hugepages-1Gi");
        assert_eq!(MemoryType::HugepagesPlain.name(), "hugepages");
    }

    #[test]
    fn allocatable_default_empty() {
        let m = ResourceManager::new();
        let a = m.allocatable();
        assert!(a.cpu_ids.is_empty() && a.memory.is_empty() && a.devices.is_empty());
    }

    #[test]
    fn validate_accepts_well_formed_pod() {
        let p = PodResources {
            name: "n".into(),
            namespace: "ns".into(),
            uid: "u".into(),
            containers: vec![
                ContainerResources {
                    name: "c1".into(),
                    cpu_ids: vec![0, 1],
                    memory: vec![],
                    devices: vec![],
                },
                ContainerResources {
                    name: "c2".into(),
                    cpu_ids: vec![2],
                    memory: vec![],
                    devices: vec![],
                },
            ],
        };
        validate_pod(&p).unwrap();
    }

    #[test]
    fn upsert_pod_with_topology_aware_devices() {
        let m = ResourceManager::new();
        m.set_allocatable(AllocatableResources {
            cpu_ids: vec![],
            memory: vec![],
            devices: vec![ContainerDevices {
                resource_name: "nvidia.com/gpu".into(),
                device_ids: vec!["g1".into()],
                topology: Some(TopologyInfo::single(0)),
            }],
        });
        let mut p = pod_simple("p1", "n", vec![]);
        p.containers[0].devices = vec![ContainerDevices {
            resource_name: "nvidia.com/gpu".into(),
            device_ids: vec!["g1".into()],
            topology: Some(TopologyInfo::single(0)),
        }];
        m.upsert_pod(p).unwrap();
        let agg = m.device_usage_by_resource();
        assert!(agg.contains_key("nvidia.com/gpu"));
    }

    #[test]
    fn remove_pod_returns_record() {
        let m = ResourceManager::new();
        m.upsert_pod(pod_simple("p1", "n", vec![])).unwrap();
        let p = m.remove_pod("p1");
        assert!(p.is_some());
    }

    #[test]
    fn remove_pod_unknown_returns_none() {
        let m = ResourceManager::new();
        assert!(m.remove_pod("ghost").is_none());
    }
}
