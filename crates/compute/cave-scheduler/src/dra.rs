// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Dynamic Resource Allocation (DRA) — KEP-4381 (GA in v1.36).
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/dynamicresources/dynamicresources.go
//!   staging/src/k8s.io/api/resource/v1/types.go (ResourceClaim, DeviceRequest, ResourceSlice)
//!
//! Pods declare ResourceClaim references; nodes publish ResourceSlices listing devices.
//! The plugin allocates devices on a candidate node and rejects nodes whose slices
//! cannot satisfy any open claim.

use crate::framework::*;
use crate::models::Node;
use std::collections::{HashMap, HashSet};

/// A resource claim — a request for K devices matching a selector. Tenant-scoped.
#[derive(Debug, Clone)]
pub struct ResourceClaim {
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub device_class: String,
    pub count: usize,
    pub selector: HashMap<String, String>,
    pub allocation: Option<DeviceAllocation>,
}

#[derive(Debug, Clone)]
pub struct DeviceAllocation {
    pub node_name: String,
    pub devices: Vec<String>, // chosen device names
}

/// A device a node publishes. Cross-node sharing is not modelled here.
#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub class: String,
    pub attributes: HashMap<String, String>,
    pub allocated_by: Option<String>, // claim_name
}

/// A node's published ResourceSlice — the set of devices on that node.
#[derive(Debug, Clone, Default)]
pub struct ResourceSlice {
    pub node_name: String,
    pub devices: Vec<Device>,
}

#[derive(Debug, Default)]
pub struct DraState {
    pub slices: HashMap<String, ResourceSlice>, // node_name → slice
    pub claims: HashMap<String, ResourceClaim>, // claim_name → claim
}

impl DraState {
    pub fn add_slice(&mut self, slice: ResourceSlice) {
        self.slices.insert(slice.node_name.clone(), slice);
    }
    pub fn add_claim(&mut self, claim: ResourceClaim) {
        self.claims.insert(claim.name.clone(), claim);
    }

    /// Find devices on `node` that satisfy `claim` (class match + attribute selector).
    /// Already-allocated devices are excluded. Returns up to claim.count names, in
    /// deterministic order, or None if not enough.
    fn try_allocate_on(&self, node: &str, claim: &ResourceClaim) -> Option<Vec<String>> {
        let slice = self.slices.get(node)?;
        let mut chosen: Vec<String> = vec![];
        let mut sorted: Vec<&Device> = slice.devices.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for dev in sorted {
            if dev.class != claim.device_class {
                continue;
            }
            if dev.allocated_by.is_some() {
                continue;
            }
            let attrs_ok = claim
                .selector
                .iter()
                .all(|(k, v)| dev.attributes.get(k) == Some(v));
            if !attrs_ok {
                continue;
            }
            chosen.push(dev.name.clone());
            if chosen.len() == claim.count {
                return Some(chosen);
            }
        }
        None
    }

    /// Persist an allocation decision. (Idempotent on the (claim, node) pair.)
    pub fn allocate(&mut self, claim_name: &str, node_name: &str, devices: Vec<String>) {
        let chosen: HashSet<String> = devices.iter().cloned().collect();
        if let Some(c) = self.claims.get_mut(claim_name) {
            c.allocation = Some(DeviceAllocation {
                node_name: node_name.into(),
                devices,
            });
        }
        if let Some(s) = self.slices.get_mut(node_name) {
            for d in &mut s.devices {
                if chosen.contains(&d.name) {
                    d.allocated_by = Some(claim_name.into());
                }
            }
        }
    }
}

pub struct DynamicResources<'a> {
    pub state: &'a DraState,
}

impl<'a> FilterPlugin for DynamicResources<'a> {
    fn name(&self) -> &str {
        "DynamicResources"
    }
    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        if pod.spec.resource_claims.is_empty() {
            return Status::skip("DynamicResources");
        }
        for r in &pod.spec.resource_claims {
            let Some(claim) = self.state.claims.get(&r.claim_name) else {
                return Status::unresolvable(
                    "DynamicResources",
                    format!("claim {} not found", r.claim_name),
                );
            };
            if claim.tenant_id != pod.tenant_id {
                return Status::unresolvable("DynamicResources", "cross-tenant claim reference");
            }
            // If already allocated to another node → reject this node.
            if let Some(alloc) = &claim.allocation {
                if alloc.node_name != node.name {
                    return Status::unschedulable(
                        "DynamicResources",
                        format!("claim {} bound to node {}", r.claim_name, alloc.node_name),
                    );
                }
                continue;
            }
            if self.state.try_allocate_on(&node.name, claim).is_none() {
                return Status::unschedulable(
                    "DynamicResources",
                    format!(
                        "node cannot satisfy claim {} ({}× {})",
                        r.claim_name, claim.count, claim.device_class
                    ),
                );
            }
        }
        Status::success("DynamicResources")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NodeStatus, ResourceCapacity};
    use chrono::Utc;
    use uuid::Uuid;

    fn ready(name: &str) -> Node {
        Node {
            name: name.into(),
            uid: Uuid::new_v4(),
            status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity {
                cpu_millicores: 1000,
                memory_bytes: 1,
                pods: 10,
                ephemeral_storage_bytes: 0,
            },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(),
            taints: vec![],
            conditions: vec![],
            registered_at: Utc::now(),
            last_heartbeat: Utc::now(),
        }
    }

    fn dev(name: &str, class: &str, vendor: &str) -> Device {
        let mut a = HashMap::new();
        a.insert("vendor".into(), vendor.into());
        Device {
            name: name.into(),
            class: class.into(),
            attributes: a,
            allocated_by: None,
        }
    }

    #[test]
    fn skip_when_no_claims() {
        let st = DraState::default();
        let plug = DynamicResources { state: &st };
        let pod = Pod::new("t", "ns", "p");
        assert_eq!(
            plug.filter(&pod, &ready("a"), &ClusterSnapshot::default())
                .code,
            Code::Skip
        );
    }

    #[test]
    fn allocates_when_devices_available() {
        let mut st = DraState::default();
        st.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![
                dev("gpu0", "nvidia.com/gpu", "nvidia"),
                dev("gpu1", "nvidia.com/gpu", "nvidia"),
            ],
        });
        st.add_claim(ResourceClaim {
            name: "c1".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            device_class: "nvidia.com/gpu".into(),
            count: 1,
            selector: HashMap::from([("vendor".into(), "nvidia".into())]),
            allocation: None,
        });
        let plug = DynamicResources { state: &st };
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.resource_claims.push(ResourceClaimRef {
            name: "claim0".into(),
            claim_name: "c1".into(),
        });
        assert!(plug
            .filter(&pod, &ready("a"), &ClusterSnapshot::default())
            .is_success());
    }

    #[test]
    fn rejects_when_class_mismatch() {
        let mut st = DraState::default();
        st.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("dev0", "intel.com/fpga", "intel")],
        });
        st.add_claim(ResourceClaim {
            name: "c1".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            device_class: "nvidia.com/gpu".into(),
            count: 1,
            selector: HashMap::new(),
            allocation: None,
        });
        let plug = DynamicResources { state: &st };
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.resource_claims.push(ResourceClaimRef {
            name: "claim0".into(),
            claim_name: "c1".into(),
        });
        assert_eq!(
            plug.filter(&pod, &ready("a"), &ClusterSnapshot::default())
                .code,
            Code::Unschedulable
        );
    }

    #[test]
    fn rejects_cross_tenant_claim() {
        let mut st = DraState::default();
        st.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("gpu0", "nvidia.com/gpu", "nvidia")],
        });
        st.add_claim(ResourceClaim {
            name: "c1".into(),
            namespace: "ns".into(),
            tenant_id: "OTHER".into(),
            device_class: "nvidia.com/gpu".into(),
            count: 1,
            selector: HashMap::new(),
            allocation: None,
        });
        let plug = DynamicResources { state: &st };
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.resource_claims.push(ResourceClaimRef {
            name: "claim0".into(),
            claim_name: "c1".into(),
        });
        let s = plug.filter(&pod, &ready("a"), &ClusterSnapshot::default());
        assert_eq!(s.code, Code::UnschedulableAndUnresolvable);
    }

    #[test]
    fn binds_node_after_allocation() {
        let mut st = DraState::default();
        st.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("gpu0", "nvidia.com/gpu", "nvidia")],
        });
        st.add_slice(ResourceSlice {
            node_name: "b".into(),
            devices: vec![dev("gpu0", "nvidia.com/gpu", "nvidia")],
        });
        st.add_claim(ResourceClaim {
            name: "c1".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            device_class: "nvidia.com/gpu".into(),
            count: 1,
            selector: HashMap::new(),
            allocation: None,
        });
        st.allocate("c1", "a", vec!["gpu0".into()]);

        let plug = DynamicResources { state: &st };
        let mut pod = Pod::new("t", "ns", "p");
        pod.spec.resource_claims.push(ResourceClaimRef {
            name: "claim0".into(),
            claim_name: "c1".into(),
        });
        // Node a (already bound) → success.
        assert!(plug
            .filter(&pod, &ready("a"), &ClusterSnapshot::default())
            .is_success());
        // Node b → unschedulable, claim is bound elsewhere.
        assert_eq!(
            plug.filter(&pod, &ready("b"), &ClusterSnapshot::default())
                .code,
            Code::Unschedulable
        );
    }
}
