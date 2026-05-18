// SPDX-License-Identifier: AGPL-3.0-or-later
//! DRA scheduler-cycle integration — Reserve / Unreserve / PreBind.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/dynamicresources/dynamicresources.go
//!   staging/src/k8s.io/api/resource/v1alpha3/types.go (ResourceClaim,
//!   DeviceClass, ResourceSlice, AllocationResult)
//!
//! Builds on `dra.rs` (Filter only) by adding the rest of the lifecycle
//! plugins. The same Filter logic is reproduced here against a Mutex-guarded
//! cluster store so allocations can be committed in PreBind and rolled back
//! in Unreserve.
//!
//! ## Cycle handshake
//!
//! 1. Filter (`DraScheduler`): for each open claim referenced by the pod,
//!    pick at least `count` matching, free devices on the candidate node;
//!    reject the node if not enough.
//! 2. Reserve: persist the picks to CycleState ("dra/state").
//! 3. PreBind: materialise — flip each claim's allocation, mark devices
//!    `allocated_by`, append a `DraBindRecord` to the audit log.
//! 4. Unreserve: revert every uncommitted allocation.

use crate::cycle_state::CycleState;
use crate::extension_points::{PreBindPlugin, ReservePlugin};
use crate::framework::{ClusterSnapshot, FilterPlugin, Pod, Status};
use crate::models::Node;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// DeviceClass — provisioner + scheduler-relevant config (no parameters yet).
///
/// Mirrors upstream `resource.k8s.io/v1alpha3.DeviceClass` with just enough
/// surface for the scheduler to: (a) mark which provisioner owns a class,
/// (b) constrain claims to a class, (c) optionally restrict suitable nodes
/// to a label selector.
#[derive(Debug, Clone)]
pub struct DeviceClass {
    pub name: String,
    pub provisioner: String,
    /// Optional `suitable_nodes` selector — nodes lacking these labels are
    /// excluded from claim allocation under this class.
    pub suitable_nodes: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
    pub class: String,
    pub attributes: HashMap<String, String>,
    pub allocated_by: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceSlice {
    pub node_name: String,
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone)]
pub struct ResourceClaim {
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub device_class: String,
    pub count: usize,
    pub selector: HashMap<String, String>,
    pub allocation: Option<AllocationResult>,
}

/// AllocationResult — node + chosen devices. Mirrors upstream
/// `AllocationResult.Devices.Results[*].Driver/Pool/Device` collapsed into
/// a flat per-claim list.
#[derive(Debug, Clone)]
pub struct AllocationResult {
    pub node_name: String,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraBindRecord {
    pub claim_name: String,
    pub node_name: String,
    pub devices: Vec<String>,
}

/// Mutable cluster-wide DRA store. Plugins query and (during PreBind) update
/// it. Designed to be shared via `Arc` across plugin lifetimes.
#[derive(Debug, Default)]
pub struct DraStore {
    pub slices: Mutex<HashMap<String, ResourceSlice>>,
    pub claims: Mutex<HashMap<String, ResourceClaim>>, // keyed by claim_name
    pub classes: Mutex<HashMap<String, DeviceClass>>,
    pub bind_log: Mutex<Vec<DraBindRecord>>,
}

impl DraStore {
    pub fn new() -> Self { Self::default() }

    pub fn add_slice(&self, slice: ResourceSlice) {
        self.slices.lock().unwrap().insert(slice.node_name.clone(), slice);
    }
    pub fn add_claim(&self, claim: ResourceClaim) {
        self.claims.lock().unwrap().insert(claim.name.clone(), claim);
    }
    pub fn add_class(&self, class: DeviceClass) {
        self.classes.lock().unwrap().insert(class.name.clone(), class);
    }
    pub fn get_claim(&self, name: &str) -> Option<ResourceClaim> {
        self.claims.lock().unwrap().get(name).cloned()
    }
    pub fn get_class(&self, name: &str) -> Option<DeviceClass> {
        self.classes.lock().unwrap().get(name).cloned()
    }
    pub fn slice_for(&self, node: &str) -> Option<ResourceSlice> {
        self.slices.lock().unwrap().get(node).cloned()
    }
    pub fn bind_records(&self) -> Vec<DraBindRecord> {
        self.bind_log.lock().unwrap().clone()
    }

    /// Reserve a set of devices on `node` to `claim_name`. Marks them
    /// `allocated_by` and writes the AllocationResult into the claim. Idempotent.
    fn commit(&self, claim_name: &str, node: &str, devices: &[String]) {
        let chosen: std::collections::HashSet<&String> = devices.iter().collect();
        if let Some(s) = self.slices.lock().unwrap().get_mut(node) {
            for d in &mut s.devices {
                if chosen.contains(&d.name) {
                    d.allocated_by = Some(claim_name.into());
                }
            }
        }
        if let Some(c) = self.claims.lock().unwrap().get_mut(claim_name) {
            c.allocation = Some(AllocationResult {
                node_name: node.into(),
                devices: devices.to_vec(),
            });
        }
        self.bind_log.lock().unwrap().push(DraBindRecord {
            claim_name: claim_name.into(),
            node_name: node.into(),
            devices: devices.to_vec(),
        });
    }

    /// Roll back a Reserve/PreBind that was never confirmed.
    fn rollback(&self, claim_name: &str, node: &str, devices: &[String]) {
        let chosen: std::collections::HashSet<&String> = devices.iter().collect();
        if let Some(s) = self.slices.lock().unwrap().get_mut(node) {
            for d in &mut s.devices {
                if chosen.contains(&d.name) && d.allocated_by.as_deref() == Some(claim_name) {
                    d.allocated_by = None;
                }
            }
        }
        if let Some(c) = self.claims.lock().unwrap().get_mut(claim_name) {
            if let Some(alloc) = &c.allocation {
                if alloc.node_name == node {
                    c.allocation = None;
                }
            }
        }
    }
}

/// Plugin-side picker — looks at one node's slice and chooses up to
/// `claim.count` devices satisfying `claim.device_class`, the device class's
/// suitable_nodes selector, and the per-attribute selector.
///
/// Returns `Some(picked)` when enough were found, else `None`.
pub fn try_allocate_on(
    store: &DraStore,
    claim: &ResourceClaim,
    node: &Node,
) -> Option<Vec<String>> {
    // Class-level node restriction (suitable_nodes labels).
    if let Some(class) = store.get_class(&claim.device_class) {
        for (k, v) in &class.suitable_nodes {
            if node.labels.get(k) != Some(v) { return None; }
        }
    }
    let slice = store.slice_for(&node.name)?;
    let mut chosen: Vec<String> = Vec::new();
    let mut sorted: Vec<&Device> = slice.devices.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));
    for dev in sorted {
        if dev.class != claim.device_class { continue; }
        if dev.allocated_by.is_some() { continue; }
        let attrs_ok = claim.selector.iter().all(|(k, v)| dev.attributes.get(k) == Some(v));
        if !attrs_ok { continue; }
        chosen.push(dev.name.clone());
        if chosen.len() == claim.count { return Some(chosen); }
    }
    None
}

/// Per-cycle DRA decisions for the framework's Reserve/Unreserve/PreBind
/// handshake.
#[derive(Debug, Clone, Default)]
pub struct DraCycleState {
    pub decisions: Vec<DraDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraDecision {
    pub claim_name: String,
    pub node_name: String,
    pub devices: Vec<String>,
}

const DRA_STATE_KEY: &str = "dra/state";

/// DRA scheduler plugin (Filter + Reserve + Unreserve + PreBind) sharing one
/// `Arc<DraStore>`.
pub struct DraScheduler {
    pub store: Arc<DraStore>,
}

impl DraScheduler {
    pub fn new(store: Arc<DraStore>) -> Self { Self { store } }
}

impl FilterPlugin for DraScheduler {
    fn name(&self) -> &str { "DraScheduler" }

    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        if pod.spec.resource_claims.is_empty() {
            return Status::skip("DraScheduler");
        }
        for r in &pod.spec.resource_claims {
            let Some(claim) = self.store.get_claim(&r.claim_name) else {
                return Status::unresolvable("DraScheduler", format!("claim {} not found", r.claim_name));
            };
            if !claim.tenant_id.is_empty() && claim.tenant_id != pod.tenant_id {
                return Status::unresolvable("DraScheduler", "cross-tenant claim reference");
            }
            // Already-allocated claim: must run on its bound node.
            if let Some(alloc) = &claim.allocation {
                if alloc.node_name != node.name {
                    return Status::unschedulable("DraScheduler",
                        format!("claim {} bound to node {}", r.claim_name, alloc.node_name));
                }
                continue;
            }
            if try_allocate_on(&self.store, &claim, node).is_none() {
                return Status::unschedulable("DraScheduler",
                    format!("node {} cannot satisfy claim {} ({}× {})",
                        node.name, r.claim_name, claim.count, claim.device_class));
            }
        }
        Status::success("DraScheduler")
    }
}

impl ReservePlugin for DraScheduler {
    fn name(&self) -> &str { "DraScheduler" }

    fn reserve(&self, pod: &Pod, node: &str, state: &CycleState) -> Status {
        if pod.spec.resource_claims.is_empty() {
            return Status::success("DraScheduler");
        }
        // Look up the node-info we need; for simplicity reserve operates with
        // the cluster store's slice for `node`.
        let node_info = Node {
            name: node.into(),
            uid: uuid::Uuid::nil(),
            status: crate::models::NodeStatus::Ready,
            capacity: crate::models::ResourceCapacity::default(),
            allocatable: crate::models::ResourceCapacity::default(),
            allocated: crate::models::ResourceCapacity::default(),
            labels: HashMap::new(),
            taints: vec![],
            conditions: vec![],
            registered_at: chrono::Utc::now(),
            last_heartbeat: chrono::Utc::now(),
        };
        let mut decisions: Vec<DraDecision> = Vec::new();
        for r in &pod.spec.resource_claims {
            let Some(claim) = self.store.get_claim(&r.claim_name) else {
                return Status::error("DraScheduler", format!("claim {} disappeared", r.claim_name));
            };
            if claim.allocation.is_some() { continue; }
            let Some(devices) = try_allocate_on(&self.store, &claim, &node_info) else {
                return Status::unschedulable("DraScheduler",
                    format!("reserve: claim {} no longer satisfiable", r.claim_name));
            };
            // Mark devices reserved immediately so concurrent claim allocations
            // in this cycle don't double-pick. Bind-log entry waits for PreBind.
            for d in &devices {
                if let Some(s) = self.store.slices.lock().unwrap().get_mut(node) {
                    for dev in &mut s.devices {
                        if &dev.name == d { dev.allocated_by = Some(r.claim_name.clone()); }
                    }
                }
            }
            decisions.push(DraDecision {
                claim_name: r.claim_name.clone(),
                node_name: node.into(),
                devices,
            });
        }
        state.write(DRA_STATE_KEY, DraCycleState { decisions });
        Status::success("DraScheduler")
    }

    fn unreserve(&self, _pod: &Pod, _node: &str, state: &CycleState) {
        let Some(cycle): Option<DraCycleState> = state.read(DRA_STATE_KEY) else { return };
        for d in &cycle.decisions {
            self.store.rollback(&d.claim_name, &d.node_name, &d.devices);
        }
        state.delete(DRA_STATE_KEY);
    }
}

impl PreBindPlugin for DraScheduler {
    fn name(&self) -> &str { "DraScheduler" }

    fn pre_bind(&self, _pod: &Pod, _node: &str, state: &CycleState) -> Status {
        let Some(cycle): Option<DraCycleState> = state.read(DRA_STATE_KEY) else {
            return Status::success("DraScheduler");
        };
        for d in &cycle.decisions {
            self.store.commit(&d.claim_name, &d.node_name, &d.devices);
        }
        Status::success("DraScheduler")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::{ClusterSnapshot, Pod, ResourceClaimRef};
    use crate::models::{Node, NodeStatus, ResourceCapacity};
    use chrono::Utc;
    use std::collections::HashSet;
    use uuid::Uuid;

    fn ready(name: &str) -> Node {
        Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity { cpu_millicores: 1000, memory_bytes: 1, pods: 10, ephemeral_storage_bytes: 0 },
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        }
    }

    fn dev(name: &str, class: &str, vendor: &str) -> Device {
        let mut a = HashMap::new();
        a.insert("vendor".into(), vendor.into());
        Device { name: name.into(), class: class.into(), attributes: a, allocated_by: None }
    }

    fn snap() -> ClusterSnapshot { ClusterSnapshot::default() }

    fn pod_with_claim(name: &str, ref_name: &str, claim_name: &str) -> Pod {
        let mut p = Pod::new("t", "ns", name);
        p.spec.resource_claims.push(ResourceClaimRef {
            name: ref_name.into(),
            claim_name: claim_name.into(),
        });
        p
    }

    fn gpu_claim(name: &str, count: usize) -> ResourceClaim {
        ResourceClaim {
            name: name.into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            device_class: "nvidia.com/gpu".into(),
            count,
            selector: HashMap::from([("vendor".into(), "nvidia".into())]),
            allocation: None,
        }
    }

    // ── Filter ────────────────────────────────────────────────────────────

    #[test]
    fn skip_when_no_claims() {
        let store = Arc::new(DraStore::new());
        let plug = DraScheduler::new(store);
        let p = Pod::new("t", "ns", "p");
        assert!(plug.filter(&p, &ready("a"), &snap()).is_skip());
    }

    #[test]
    fn allocates_when_devices_match() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("gpu0", "nvidia.com/gpu", "nvidia"), dev("gpu1", "nvidia.com/gpu", "nvidia")],
        });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store);
        let p = pod_with_claim("p", "claim0", "c1");
        assert!(plug.filter(&p, &ready("a"), &snap()).is_success());
    }

    #[test]
    fn rejects_when_class_mismatch() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("d", "intel.com/fpga", "intel")],
        });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store);
        let p = pod_with_claim("p", "claim0", "c1");
        assert!(plug.filter(&p, &ready("a"), &snap()).is_rejected());
    }

    #[test]
    fn rejects_cross_tenant_claim() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("g", "nvidia.com/gpu", "nvidia")],
        });
        let mut c = gpu_claim("c1", 1);
        c.tenant_id = "OTHER".into();
        store.add_claim(c);
        let plug = DraScheduler::new(store);
        let p = pod_with_claim("p", "claim0", "c1");
        assert_eq!(plug.filter(&p, &ready("a"), &snap()).code, crate::framework::Code::UnschedulableAndUnresolvable);
    }

    #[test]
    fn missing_claim_unresolvable() {
        let store = Arc::new(DraStore::new());
        let plug = DraScheduler::new(store);
        let p = pod_with_claim("p", "claim0", "ghost");
        assert_eq!(plug.filter(&p, &ready("a"), &snap()).code, crate::framework::Code::UnschedulableAndUnresolvable);
    }

    #[test]
    fn already_bound_pinned_to_node() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![dev("g", "nvidia.com/gpu", "nvidia")] });
        store.add_slice(ResourceSlice { node_name: "b".into(), devices: vec![dev("g", "nvidia.com/gpu", "nvidia")] });
        let mut c = gpu_claim("c1", 1);
        c.allocation = Some(AllocationResult { node_name: "a".into(), devices: vec!["g".into()] });
        store.add_claim(c);
        let plug = DraScheduler::new(store);
        let p = pod_with_claim("p", "claim0", "c1");
        assert!(plug.filter(&p, &ready("a"), &snap()).is_success());
        assert!(plug.filter(&p, &ready("b"), &snap()).is_rejected());
    }

    #[test]
    fn class_suitable_nodes_excludes_unmatched() {
        let store = Arc::new(DraStore::new());
        store.add_class(DeviceClass {
            name: "nvidia.com/gpu".into(),
            provisioner: "test".into(),
            suitable_nodes: HashMap::from([("gpu".into(), "true".into())]),
        });
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![dev("g", "nvidia.com/gpu", "nvidia")] });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store);
        let p = pod_with_claim("p", "claim0", "c1");
        let mut a = ready("a");
        // Without label gpu=true → reject.
        assert!(plug.filter(&p, &a, &snap()).is_rejected());
        a.labels.insert("gpu".into(), "true".into());
        assert!(plug.filter(&p, &a, &snap()).is_success());
    }

    #[test]
    fn allocator_skips_already_allocated_devices() {
        let store = Arc::new(DraStore::new());
        let mut d1 = dev("g0", "nvidia.com/gpu", "nvidia"); d1.allocated_by = Some("other".into());
        let d2 = dev("g1", "nvidia.com/gpu", "nvidia");
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![d1, d2] });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store.clone());
        let p = pod_with_claim("p", "c", "c1");
        // g0 taken; g1 free → should succeed.
        assert!(plug.filter(&p, &ready("a"), &snap()).is_success());
        // Need 2 → only 1 free → reject.
        store.claims.lock().unwrap().get_mut("c1").unwrap().count = 2;
        assert!(plug.filter(&p, &ready("a"), &snap()).is_rejected());
    }

    #[test]
    fn allocator_attribute_selector_filters() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![
            dev("g0", "nvidia.com/gpu", "nvidia"),
            dev("g1", "nvidia.com/gpu", "amd"),
        ]});
        let mut c = gpu_claim("c1", 1);
        c.selector = HashMap::from([("vendor".into(), "nvidia".into())]);
        store.add_claim(c);
        let plug = DraScheduler::new(store.clone());
        let p = pod_with_claim("p", "c", "c1");
        assert!(plug.filter(&p, &ready("a"), &snap()).is_success());
        // Now make only AMD remaining.
        store.slices.lock().unwrap().get_mut("a").unwrap().devices[0].allocated_by = Some("x".into());
        assert!(plug.filter(&p, &ready("a"), &snap()).is_rejected());
    }

    // ── Reserve / Unreserve / PreBind ─────────────────────────────────────

    #[test]
    fn reserve_marks_devices_and_pre_bind_commits() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![dev("g0", "nvidia.com/gpu", "nvidia")] });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store.clone());
        let cs = CycleState::new();
        let p = pod_with_claim("p", "c", "c1");

        let s = ReservePlugin::reserve(&plug, &p, "a", &cs);
        assert!(s.is_success());
        // Reserve marks device allocated_by claim, so a second allocation cannot reuse it.
        let dev_now = store.slices.lock().unwrap().get("a").unwrap().devices[0].clone();
        assert_eq!(dev_now.allocated_by.as_deref(), Some("c1"));
        // But claim.allocation is still empty until PreBind.
        assert!(store.get_claim("c1").unwrap().allocation.is_none());

        let s2 = PreBindPlugin::pre_bind(&plug, &p, "a", &cs);
        assert!(s2.is_success());
        let claim = store.get_claim("c1").unwrap();
        let alloc = claim.allocation.unwrap();
        assert_eq!(alloc.node_name, "a");
        assert_eq!(alloc.devices, vec!["g0".to_string()]);
        let log = store.bind_records();
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn unreserve_releases_devices() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![dev("g0", "nvidia.com/gpu", "nvidia")] });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store.clone());
        let cs = CycleState::new();
        let p = pod_with_claim("p", "c", "c1");

        ReservePlugin::reserve(&plug, &p, "a", &cs);
        ReservePlugin::unreserve(&plug, &p, "a", &cs);
        let dev_now = store.slices.lock().unwrap().get("a").unwrap().devices[0].clone();
        assert!(dev_now.allocated_by.is_none());
        // CycleState entry is wiped.
        let cycle: Option<DraCycleState> = cs.read(DRA_STATE_KEY);
        assert!(cycle.is_none());
    }

    #[test]
    fn reserve_skips_already_allocated_claim() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![dev("g0", "nvidia.com/gpu", "nvidia")] });
        let mut c = gpu_claim("c1", 1);
        c.allocation = Some(AllocationResult { node_name: "a".into(), devices: vec!["pre".into()] });
        store.add_claim(c);
        let plug = DraScheduler::new(store.clone());
        let cs = CycleState::new();
        let p = pod_with_claim("p", "c", "c1");
        let s = ReservePlugin::reserve(&plug, &p, "a", &cs);
        assert!(s.is_success());
        let cycle: DraCycleState = cs.read(DRA_STATE_KEY).unwrap();
        assert_eq!(cycle.decisions.len(), 0);
    }

    #[test]
    fn pre_bind_no_state_is_no_op() {
        let store = Arc::new(DraStore::new());
        let plug = DraScheduler::new(store.clone());
        let cs = CycleState::new();
        let p = Pod::new("t", "ns", "p");
        assert!(PreBindPlugin::pre_bind(&plug, &p, "a", &cs).is_success());
        assert_eq!(store.bind_records().len(), 0);
    }

    #[test]
    fn reserve_fails_when_no_devices_remain() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![] });
        store.add_claim(gpu_claim("c1", 1));
        let plug = DraScheduler::new(store);
        let cs = CycleState::new();
        let p = pod_with_claim("p", "c", "c1");
        assert!(ReservePlugin::reserve(&plug, &p, "a", &cs).is_rejected());
    }

    // ── DraStore bookkeeping ──────────────────────────────────────────────

    #[test]
    fn store_records_audit_log_in_order() {
        let store = Arc::new(DraStore::new());
        store.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("g0", "nvidia.com/gpu", "nvidia"), dev("g1", "nvidia.com/gpu", "nvidia")],
        });
        store.add_claim(gpu_claim("c1", 1));
        store.add_claim(gpu_claim("c2", 1));
        let plug = DraScheduler::new(store.clone());
        let cs = CycleState::new();
        let mut p = Pod::new("t", "ns", "p");
        p.spec.resource_claims.push(ResourceClaimRef { name: "r1".into(), claim_name: "c1".into() });
        p.spec.resource_claims.push(ResourceClaimRef { name: "r2".into(), claim_name: "c2".into() });
        ReservePlugin::reserve(&plug, &p, "a", &cs);
        PreBindPlugin::pre_bind(&plug, &p, "a", &cs);
        let log = store.bind_records();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].claim_name, "c1");
        assert_eq!(log[1].claim_name, "c2");
    }

    #[test]
    fn try_allocate_returns_none_when_class_mismatch() {
        let store = DraStore::new();
        store.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![dev("d", "intel.com/fpga", "intel")],
        });
        let claim = gpu_claim("c1", 1);
        let n = ready("a");
        assert!(try_allocate_on(&store, &claim, &n).is_none());
    }

    #[test]
    fn try_allocate_picks_deterministic_first_match() {
        let store = DraStore::new();
        store.add_slice(ResourceSlice {
            node_name: "a".into(),
            devices: vec![
                dev("zlast", "nvidia.com/gpu", "nvidia"),
                dev("aaa", "nvidia.com/gpu", "nvidia"),
                dev("mmm", "nvidia.com/gpu", "nvidia"),
            ],
        });
        let claim = gpu_claim("c1", 1);
        let n = ready("a");
        let picked = try_allocate_on(&store, &claim, &n).unwrap();
        // Sorted by name → "aaa" first.
        assert_eq!(picked, vec!["aaa".to_string()]);
    }

    #[test]
    fn allocation_result_carries_devices() {
        let r = AllocationResult { node_name: "n".into(), devices: vec!["d1".into(), "d2".into()] };
        assert_eq!(r.devices.len(), 2);
        assert_eq!(r.node_name, "n");
    }

    #[test]
    fn device_class_round_trip() {
        let store = DraStore::new();
        store.add_class(DeviceClass {
            name: "x".into(), provisioner: "p".into(),
            suitable_nodes: HashMap::from([("zone".into(), "us-east-1a".into())]),
        });
        let c = store.get_class("x").unwrap();
        assert_eq!(c.provisioner, "p");
        assert_eq!(c.suitable_nodes.get("zone").unwrap(), "us-east-1a");
        assert!(store.get_class("y").is_none());
    }

    #[test]
    fn dra_decision_equality_for_audit() {
        let a = DraDecision { claim_name: "c1".into(), node_name: "n".into(), devices: vec!["d".into()] };
        let b = a.clone();
        let mut s = HashSet::new();
        s.insert(format!("{:?}", a));
        s.insert(format!("{:?}", b));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn rollback_only_clears_self_owned_devices() {
        let store = DraStore::new();
        let mut d = dev("g", "nvidia.com/gpu", "nvidia");
        d.allocated_by = Some("other".into());
        store.add_slice(ResourceSlice { node_name: "a".into(), devices: vec![d] });
        store.rollback("self", "a", &["g".into()]);
        // Other-owner should not be cleared.
        assert_eq!(
            store.slice_for("a").unwrap().devices[0].allocated_by.as_deref(),
            Some("other"),
        );
    }
}
