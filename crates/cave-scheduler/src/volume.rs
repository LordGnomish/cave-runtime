//! Volume scheduling deeper — VolumeBinding (Immediate / WaitForFirstConsumer
//! / late-binding handshake) and VolumeZone.
//!
//! Cite: kubernetes/kubernetes v1.31.0
//!   pkg/scheduler/framework/plugins/volumebinding/volume_binding.go
//!   pkg/scheduler/framework/plugins/volumezone/volume_zone.go
//!   staging/src/k8s.io/api/storage/v1/types.go (StorageClass)
//!
//! ## Late-binding handshake
//!
//! For a PVC with `WaitForFirstConsumer` storage class:
//!   1. Filter: pick a node satisfying the PV's allowed topologies.
//!   2. Reserve: record the (pvc, node) binding decision in the per-cycle
//!      `VolumeBindingState` (in CycleState under "volumebinding/state").
//!   3. PreBind: materialise the decision — flip the PVC's bound_node and
//!      append to the persistent log so observers can see the binding.
//!   4. Unreserve: roll the in-progress binding back when a downstream
//!      plugin rejects the cycle.

use crate::cycle_state::CycleState;
use crate::extension_points::{PreBindPlugin, ReservePlugin};
use crate::framework::{ClusterSnapshot, FilterPlugin, Pod, ScorePlugin, Status, MAX_NODE_SCORE};
use crate::models::Node;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

/// PV access mode — mirrors corev1.PersistentVolumeAccessMode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessMode {
    ReadWriteOnce,
    ReadOnlyMany,
    ReadWriteMany,
    /// KEP-2485 GA in v1.29.
    ReadWriteOncePod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeBindingMode {
    Immediate,
    WaitForFirstConsumer,
}

/// StorageClass — only the scheduler-relevant fields.
#[derive(Debug, Clone)]
pub struct StorageClass {
    pub name: String,
    pub provisioner: String,
    pub volume_binding_mode: VolumeBindingMode,
    /// `allowed_topologies` — list of topology terms; the scheduler must pick
    /// a node whose labels satisfy at least one term. Each term is a map
    /// `topology_key → allowed_values`.
    pub allowed_topologies: Vec<HashMap<String, Vec<String>>>,
}

/// PersistentVolume — pre-provisioned or dynamically provisioned.
#[derive(Debug, Clone, Default)]
pub struct PersistentVolume {
    pub name: String,
    pub storage_class: Option<String>,
    pub access_modes: HashSet<AccessMode>,
    pub capacity_bytes: u64,
    /// node-affinity-style topology requirement (e.g. zone=us-east-1a).
    pub node_affinity: HashMap<String, Vec<String>>,
    /// Bound PVC (one PVC per PV). `None` until the binder writes it.
    pub claim_ref: Option<String>,
}

/// PersistentVolumeClaim — what a pod requests.
#[derive(Debug, Clone, Default)]
pub struct PersistentVolumeClaim {
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub storage_class: Option<String>,
    pub access_modes: HashSet<AccessMode>,
    pub requested_bytes: u64,
    /// Once bound, the chosen PV. `None` until the binder writes it.
    pub volume_name: Option<String>,
    /// Once a node is selected (WFC late-binding), records that decision.
    pub bound_node: Option<String>,
}

/// Mutable scheduler-side store of PV / PVC / StorageClass facts. Plugins
/// query and (during PreBind) update this.
#[derive(Debug, Default)]
pub struct VolumeStore {
    pub pvs: Mutex<HashMap<String, PersistentVolume>>,
    pub pvcs: Mutex<HashMap<String, PersistentVolumeClaim>>,
    pub storage_classes: Mutex<HashMap<String, StorageClass>>,
    /// Audit log of every materialised binding (pvc_key, node, pv_name).
    pub bind_log: Mutex<Vec<BindRecord>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindRecord {
    pub pvc_key: String,
    pub node_name: String,
    pub pv_name: Option<String>,
}

impl VolumeStore {
    pub fn new() -> Self { Self::default() }

    pub fn add_pv(&self, pv: PersistentVolume) {
        self.pvs.lock().unwrap().insert(pv.name.clone(), pv);
    }
    pub fn add_pvc(&self, pvc: PersistentVolumeClaim) {
        let key = format!("{}/{}", pvc.namespace, pvc.name);
        self.pvcs.lock().unwrap().insert(key, pvc);
    }
    pub fn add_storage_class(&self, sc: StorageClass) {
        self.storage_classes.lock().unwrap().insert(sc.name.clone(), sc);
    }

    pub fn get_pvc(&self, ns: &str, name: &str) -> Option<PersistentVolumeClaim> {
        self.pvcs.lock().unwrap().get(&format!("{}/{}", ns, name)).cloned()
    }

    pub fn get_storage_class(&self, name: &str) -> Option<StorageClass> {
        self.storage_classes.lock().unwrap().get(name).cloned()
    }

    pub fn get_pv(&self, name: &str) -> Option<PersistentVolume> {
        self.pvs.lock().unwrap().get(name).cloned()
    }

    pub fn bind_records(&self) -> Vec<BindRecord> {
        self.bind_log.lock().unwrap().clone()
    }

    /// Materialise a Reserve decision: flip pvc.bound_node, optionally bind a PV.
    fn commit_binding(&self, pvc_key: &str, node: &str, pv_name: Option<&str>) {
        if let Some(pvc) = self.pvcs.lock().unwrap().get_mut(pvc_key) {
            pvc.bound_node = Some(node.into());
            if let Some(p) = pv_name {
                pvc.volume_name = Some(p.into());
            }
        }
        if let Some(pv) = pv_name {
            if let Some(pv_entry) = self.pvs.lock().unwrap().get_mut(pv) {
                pv_entry.claim_ref = Some(pvc_key.into());
            }
        }
        self.bind_log.lock().unwrap().push(BindRecord {
            pvc_key: pvc_key.into(),
            node_name: node.into(),
            pv_name: pv_name.map(Into::into),
        });
    }

    /// Roll back a Reserve that was never PreBind'd. Removes pvc.bound_node
    /// and any PV.claim_ref written this cycle.
    fn rollback_binding(&self, pvc_key: &str) {
        if let Some(pvc) = self.pvcs.lock().unwrap().get_mut(pvc_key) {
            let pv_name = pvc.volume_name.clone();
            pvc.bound_node = None;
            pvc.volume_name = None;
            if let Some(pv) = pv_name {
                if let Some(pv_entry) = self.pvs.lock().unwrap().get_mut(&pv) {
                    if pv_entry.claim_ref.as_deref() == Some(pvc_key) {
                        pv_entry.claim_ref = None;
                    }
                }
            }
        }
    }
}

/// Per-cycle binding decisions that the framework must roll back if a later
/// plugin rejects the cycle. Keyed by pvc_key.
#[derive(Debug, Clone, Default)]
pub struct VolumeBindingCycleState {
    pub decisions: Vec<(String, String, Option<String>)>, // (pvc_key, node, pv_name)
}

/// CycleState key for per-cycle binding decisions.
const VOLUME_BINDING_STATE_KEY: &str = "volumebinding/state";

/// Look up the PVC referenced by a pod volume; returns Err on
/// missing-PVC (UnschedulableAndUnresolvable).
fn lookup_pvc(
    pod: &Pod,
    pvc_name: &str,
    store: &VolumeStore,
) -> Result<PersistentVolumeClaim, Status> {
    let pvc = store.get_pvc(&pod.namespace, pvc_name).ok_or_else(|| {
        Status::unresolvable("VolumeBinding", format!("pvc {}/{} not found", pod.namespace, pvc_name))
    })?;
    if pvc.tenant_id != pod.tenant_id && !pvc.tenant_id.is_empty() {
        return Err(Status::unresolvable("VolumeBinding", "cross-tenant PVC reference"));
    }
    Ok(pvc)
}

/// Check `node.labels` against an allowed-topologies term map. Returns true
/// when every (key, allowed_values) entry is satisfied.
fn node_matches_topology(node: &Node, term: &HashMap<String, Vec<String>>) -> bool {
    for (k, allowed) in term {
        let Some(v) = node.labels.get(k) else { return false; };
        if !allowed.iter().any(|x| x == v) { return false; }
    }
    true
}

/// VolumeBinding deeper — Filter + Reserve + PreBind + Unreserve.
///
/// Required state: `Arc<VolumeStore>` (cluster-wide PV/PVC/SC store).
pub struct VolumeBinding {
    pub store: std::sync::Arc<VolumeStore>,
}

impl VolumeBinding {
    pub fn new(store: std::sync::Arc<VolumeStore>) -> Self { Self { store } }
}

impl FilterPlugin for VolumeBinding {
    fn name(&self) -> &str { "VolumeBinding" }

    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        for v in &pod.spec.volumes {
            let crate::framework::VolumeKind::PersistentVolumeClaim { claim_name, bound_node } = &v.kind else { continue; };

            // Legacy direct bound_node short-circuit (kept for backwards compat
            // with pods that already encode the binding inline).
            if let Some(bn) = bound_node {
                if bn != &node.name {
                    return Status::unschedulable("VolumeBinding", format!("PVC {} bound to {}", claim_name, bn));
                }
                continue;
            }

            let pvc = match lookup_pvc(pod, claim_name, &self.store) {
                Ok(p) => p,
                Err(s) => return s,
            };

            // Already-bound PVC: must run on the same node.
            if let Some(existing_node) = &pvc.bound_node {
                if existing_node != &node.name {
                    return Status::unschedulable("VolumeBinding",
                        format!("PVC {} bound to {}", claim_name, existing_node));
                }
                continue;
            }

            // Storage class interaction.
            let sc = pvc.storage_class.as_ref()
                .and_then(|n| self.store.get_storage_class(n));

            match sc.as_ref().map(|s| s.volume_binding_mode) {
                // Immediate: PVC must already be bound to a PV.
                Some(VolumeBindingMode::Immediate) => {
                    let Some(pv_name) = &pvc.volume_name else {
                        return Status::unschedulable("VolumeBinding",
                            format!("immediate PVC {} not yet bound", claim_name));
                    };
                    let pv = match self.store.get_pv(pv_name) {
                        Some(p) => p,
                        None => return Status::unresolvable("VolumeBinding",
                            format!("PV {} missing for immediate PVC {}", pv_name, claim_name)),
                    };
                    if !pv.node_affinity.is_empty() && !node_matches_topology(node, &pv.node_affinity) {
                        return Status::unschedulable("VolumeBinding",
                            format!("PV {} node-affinity excludes {}", pv_name, node.name));
                    }
                    if let Some(claim_user) = &pv.claim_ref {
                        let want = format!("{}/{}", pvc.namespace, pvc.name);
                        if claim_user != &want {
                            return Status::unschedulable("VolumeBinding",
                                format!("PV {} bound to a different PVC {}", pv_name, claim_user));
                        }
                    }
                }
                // WaitForFirstConsumer or no SC: node must satisfy the SC's
                // allowed_topologies if any. If a PV is already provisioned
                // and node-affinity-restricted, also enforce that.
                Some(VolumeBindingMode::WaitForFirstConsumer) | None => {
                    if let Some(s) = &sc {
                        if !s.allowed_topologies.is_empty() {
                            let any = s.allowed_topologies.iter().any(|t| node_matches_topology(node, t));
                            if !any {
                                return Status::unschedulable("VolumeBinding",
                                    format!("node {} not in allowed_topologies of class {}", node.name, s.name));
                            }
                        }
                    }
                    if let Some(pv_name) = &pvc.volume_name {
                        if let Some(pv) = self.store.get_pv(pv_name) {
                            if !pv.node_affinity.is_empty() && !node_matches_topology(node, &pv.node_affinity) {
                                return Status::unschedulable("VolumeBinding",
                                    format!("PV {} node-affinity excludes {}", pv_name, node.name));
                            }
                        }
                    }
                }
            }
        }
        Status::success("VolumeBinding")
    }
}

impl ReservePlugin for VolumeBinding {
    fn name(&self) -> &str { "VolumeBinding" }

    fn reserve(&self, pod: &Pod, node: &str, state: &CycleState) -> Status {
        let mut decisions: Vec<(String, String, Option<String>)> = Vec::new();
        for v in &pod.spec.volumes {
            let crate::framework::VolumeKind::PersistentVolumeClaim { claim_name, bound_node } = &v.kind else { continue; };
            // Legacy literal bound_node — assumed already materialised.
            if bound_node.is_some() { continue; }
            let pvc = match lookup_pvc(pod, claim_name, &self.store) {
                Ok(p) => p,
                Err(s) => return s,
            };
            if pvc.bound_node.is_some() { continue; }
            // Decision: pin pvc to this node. Optionally bind a matching PV.
            let pv_name = pvc.volume_name.clone();
            decisions.push((format!("{}/{}", pvc.namespace, pvc.name), node.into(), pv_name));
        }
        state.write(VOLUME_BINDING_STATE_KEY, VolumeBindingCycleState { decisions });
        Status::success("VolumeBinding")
    }

    fn unreserve(&self, _pod: &Pod, _node: &str, state: &CycleState) {
        let Some(cycle): Option<VolumeBindingCycleState> = state.read(VOLUME_BINDING_STATE_KEY) else { return };
        for (pvc_key, _, _) in cycle.decisions {
            self.store.rollback_binding(&pvc_key);
        }
        state.delete(VOLUME_BINDING_STATE_KEY);
    }
}

impl PreBindPlugin for VolumeBinding {
    fn name(&self) -> &str { "VolumeBinding" }

    fn pre_bind(&self, _pod: &Pod, _node: &str, state: &CycleState) -> Status {
        let Some(cycle): Option<VolumeBindingCycleState> = state.read(VOLUME_BINDING_STATE_KEY) else {
            return Status::success("VolumeBinding");
        };
        for (pvc_key, node, pv_name) in &cycle.decisions {
            self.store.commit_binding(pvc_key, node, pv_name.as_deref());
        }
        Status::success("VolumeBinding")
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VolumeZone — Filter only.
//
// Cite: pkg/scheduler/framework/plugins/volumezone/volume_zone.go
//
// Walks the pod's PVCs; for each bound PV with a topology key in the
// well-known set (zone, region), the node's matching label must be present
// in the PV's allowed values.

const ZONE_LABELS: &[&str] = &[
    "topology.kubernetes.io/zone",
    "topology.kubernetes.io/region",
    "failure-domain.beta.kubernetes.io/zone",
    "failure-domain.beta.kubernetes.io/region",
];

pub struct VolumeZone {
    pub store: std::sync::Arc<VolumeStore>,
}

impl VolumeZone {
    pub fn new(store: std::sync::Arc<VolumeStore>) -> Self { Self { store } }
}

impl FilterPlugin for VolumeZone {
    fn name(&self) -> &str { "VolumeZone" }

    fn filter(&self, pod: &Pod, node: &Node, _: &ClusterSnapshot) -> Status {
        for v in &pod.spec.volumes {
            let crate::framework::VolumeKind::PersistentVolumeClaim { claim_name, bound_node: _ } = &v.kind else { continue; };
            let Some(pvc) = self.store.get_pvc(&pod.namespace, claim_name) else { continue; };
            let Some(pv_name) = &pvc.volume_name else { continue; };
            let Some(pv) = self.store.get_pv(pv_name) else { continue; };
            for label in ZONE_LABELS {
                let Some(allowed) = pv.node_affinity.get(*label) else { continue; };
                let Some(node_val) = node.labels.get(*label) else {
                    return Status::unschedulable("VolumeZone",
                        format!("node lacks {} required by PV {}", label, pv_name));
                };
                if !allowed.iter().any(|x| x == node_val) {
                    return Status::unschedulable("VolumeZone",
                        format!("node {}={} not in PV {} allowed zones", label, node_val, pv_name));
                }
            }
        }
        Status::success("VolumeZone")
    }
}

impl ScorePlugin for VolumeZone {
    fn name(&self) -> &str { "VolumeZone" }
    fn score(&self, _pod: &Pod, _node: &Node, _: &ClusterSnapshot) -> i64 {
        // VolumeZone is a hard predicate; score plugin is a no-op (every
        // node that passes Filter scores the max).
        MAX_NODE_SCORE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::{Pod, VolumeKind, VolumeSpec};
    use crate::models::{NodeStatus, ResourceCapacity};
    use chrono::Utc;
    use std::sync::Arc;
    use uuid::Uuid;

    fn node_with_labels(name: &str, labels: &[(&str, &str)]) -> Node {
        let mut node = Node {
            name: name.into(), uid: Uuid::new_v4(), status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity::default(),
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(), taints: vec![], conditions: vec![],
            registered_at: Utc::now(), last_heartbeat: Utc::now(),
        };
        for (k, v) in labels { node.labels.insert((*k).into(), (*v).into()); }
        node
    }

    fn pvc(name: &str, sc: Option<&str>) -> PersistentVolumeClaim {
        PersistentVolumeClaim {
            name: name.into(), namespace: "ns".into(), tenant_id: "t".into(),
            storage_class: sc.map(Into::into),
            access_modes: HashSet::from([AccessMode::ReadWriteOnce]),
            requested_bytes: 1_000_000_000,
            volume_name: None, bound_node: None,
        }
    }

    fn pod_with_pvc(name: &str, claim_name: &str) -> Pod {
        let mut p = Pod::new("t", "ns", name);
        p.spec.volumes.push(VolumeSpec {
            name: "v".into(),
            kind: VolumeKind::PersistentVolumeClaim { claim_name: claim_name.into(), bound_node: None },
        });
        p
    }

    fn snap() -> ClusterSnapshot {
        ClusterSnapshot { nodes: vec![], pods_by_node: HashMap::new() }
    }

    // ── VolumeBinding Filter ──────────────────────────────────────────────

    #[test]
    fn missing_pvc_is_unresolvable() {
        let store = Arc::new(VolumeStore::new());
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "ghost");
        let s = plug.filter(&p, &node_with_labels("a", &[]), &snap());
        assert_eq!(s.code, crate::framework::Code::UnschedulableAndUnresolvable);
    }

    #[test]
    fn cross_tenant_pvc_unresolvable() {
        let store = Arc::new(VolumeStore::new());
        let mut c = pvc("c", None);
        c.tenant_id = "other".into();
        store.add_pvc(c);
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        let s = plug.filter(&p, &node_with_labels("a", &[]), &snap());
        assert_eq!(s.code, crate::framework::Code::UnschedulableAndUnresolvable);
    }

    #[test]
    fn already_bound_pvc_to_different_node_rejected() {
        let store = Arc::new(VolumeStore::new());
        let mut c = pvc("c", None);
        c.bound_node = Some("other".into());
        store.add_pvc(c);
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        let s = plug.filter(&p, &node_with_labels("a", &[]), &snap());
        assert!(s.is_rejected());
    }

    #[test]
    fn already_bound_pvc_to_same_node_succeeds() {
        let store = Arc::new(VolumeStore::new());
        let mut c = pvc("c", None);
        c.bound_node = Some("a".into());
        store.add_pvc(c);
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        assert!(plug.filter(&p, &node_with_labels("a", &[]), &snap()).is_success());
    }

    #[test]
    fn immediate_unbound_pvc_is_unschedulable() {
        let store = Arc::new(VolumeStore::new());
        store.add_storage_class(StorageClass {
            name: "fast".into(),
            provisioner: "test".into(),
            volume_binding_mode: VolumeBindingMode::Immediate,
            allowed_topologies: vec![],
        });
        store.add_pvc(pvc("c", Some("fast")));
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        let s = plug.filter(&p, &node_with_labels("a", &[]), &snap());
        assert!(s.is_rejected());
        assert!(s.reasons[0].contains("immediate"));
    }

    #[test]
    fn immediate_with_pv_node_affinity_filters() {
        let store = Arc::new(VolumeStore::new());
        store.add_storage_class(StorageClass {
            name: "fast".into(),
            provisioner: "test".into(),
            volume_binding_mode: VolumeBindingMode::Immediate,
            allowed_topologies: vec![],
        });
        let mut pv = PersistentVolume::default();
        pv.name = "pv0".into();
        pv.node_affinity.insert("zone".into(), vec!["us-east-1a".into()]);
        store.add_pv(pv);
        let mut c = pvc("c", Some("fast"));
        c.volume_name = Some("pv0".into());
        store.add_pvc(c);

        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        // Wrong zone → reject.
        let s = plug.filter(&p, &node_with_labels("a", &[("zone", "us-east-1b")]), &snap());
        assert!(s.is_rejected());
        // Right zone → success.
        assert!(plug.filter(&p, &node_with_labels("a", &[("zone", "us-east-1a")]), &snap()).is_success());
    }

    #[test]
    fn wfc_unbound_pvc_succeeds_when_topology_allowed() {
        let store = Arc::new(VolumeStore::new());
        store.add_storage_class(StorageClass {
            name: "wfc".into(),
            provisioner: "test".into(),
            volume_binding_mode: VolumeBindingMode::WaitForFirstConsumer,
            allowed_topologies: vec![
                HashMap::from([("zone".into(), vec!["us-east-1a".into(), "us-east-1b".into()])]),
            ],
        });
        store.add_pvc(pvc("c", Some("wfc")));

        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        // Allowed zone.
        assert!(plug.filter(&p, &node_with_labels("a", &[("zone", "us-east-1a")]), &snap()).is_success());
        // Disallowed zone.
        assert!(plug.filter(&p, &node_with_labels("b", &[("zone", "eu-west-1")]), &snap()).is_rejected());
    }

    #[test]
    fn wfc_no_storage_class_succeeds() {
        let store = Arc::new(VolumeStore::new());
        store.add_pvc(pvc("c", None));
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        assert!(plug.filter(&p, &node_with_labels("a", &[]), &snap()).is_success());
    }

    // ── VolumeBinding late-binding handshake (Reserve + PreBind + Unreserve) ──

    #[test]
    fn reserve_records_decision_and_pre_bind_commits() {
        let store = Arc::new(VolumeStore::new());
        store.add_pvc(pvc("c", None));
        let plug = VolumeBinding::new(store.clone());
        let cs = CycleState::new();
        let p = pod_with_pvc("p", "c");

        let s = ReservePlugin::reserve(&plug, &p, "node-1", &cs);
        assert!(s.is_success());
        // Reserve doesn't commit yet — pvc still unbound.
        let pre = store.get_pvc("ns", "c").unwrap();
        assert!(pre.bound_node.is_none());

        // PreBind materialises.
        let s2 = PreBindPlugin::pre_bind(&plug, &p, "node-1", &cs);
        assert!(s2.is_success());
        let post = store.get_pvc("ns", "c").unwrap();
        assert_eq!(post.bound_node.as_deref(), Some("node-1"));
        let log = store.bind_records();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].pvc_key, "ns/c");
    }

    #[test]
    fn unreserve_rolls_back_committed_binding() {
        let store = Arc::new(VolumeStore::new());
        store.add_pvc(pvc("c", None));
        let plug = VolumeBinding::new(store.clone());
        let cs = CycleState::new();
        let p = pod_with_pvc("p", "c");

        ReservePlugin::reserve(&plug, &p, "node-1", &cs);
        PreBindPlugin::pre_bind(&plug, &p, "node-1", &cs);
        // Then a downstream Bind plugin fails, so the framework calls unreserve.
        ReservePlugin::unreserve(&plug, &p, "node-1", &cs);
        let post = store.get_pvc("ns", "c").unwrap();
        assert!(post.bound_node.is_none());
    }

    #[test]
    fn reserve_idempotent_when_pvc_already_bound() {
        let store = Arc::new(VolumeStore::new());
        let mut c = pvc("c", None);
        c.bound_node = Some("node-1".into());
        store.add_pvc(c);
        let plug = VolumeBinding::new(store.clone());
        let cs = CycleState::new();
        let p = pod_with_pvc("p", "c");
        let s = ReservePlugin::reserve(&plug, &p, "node-1", &cs);
        assert!(s.is_success());
        // No new decisions appended (pvc was already bound).
        let cycle: VolumeBindingCycleState = cs.read(VOLUME_BINDING_STATE_KEY).unwrap();
        assert_eq!(cycle.decisions.len(), 0);
    }

    #[test]
    fn pre_bind_no_op_when_no_state() {
        let store = Arc::new(VolumeStore::new());
        let plug = VolumeBinding::new(store.clone());
        let cs = CycleState::new();
        let p = Pod::new("t", "ns", "p"); // no volumes
        let s = PreBindPlugin::pre_bind(&plug, &p, "n", &cs);
        assert!(s.is_success());
        assert_eq!(store.bind_records().len(), 0);
    }

    // ── AccessMode coverage ──────────────────────────────────────────────

    #[test]
    fn access_modes_distinguishable() {
        // Sanity: enum values are distinct hashes / equality.
        let mut s: HashSet<AccessMode> = HashSet::new();
        s.insert(AccessMode::ReadWriteOnce);
        s.insert(AccessMode::ReadOnlyMany);
        s.insert(AccessMode::ReadWriteMany);
        s.insert(AccessMode::ReadWriteOncePod);
        assert_eq!(s.len(), 4);
    }

    // ── VolumeZone Filter ─────────────────────────────────────────────────

    fn make_zoned_pv(name: &str, zone: &str) -> PersistentVolume {
        let mut pv = PersistentVolume::default();
        pv.name = name.into();
        pv.node_affinity.insert(
            "topology.kubernetes.io/zone".into(),
            vec![zone.into()],
        );
        pv
    }

    #[test]
    fn volume_zone_passes_when_zone_matches() {
        let store = Arc::new(VolumeStore::new());
        store.add_pv(make_zoned_pv("pv0", "us-east-1a"));
        let mut c = pvc("c", None); c.volume_name = Some("pv0".into());
        store.add_pvc(c);
        let plug = VolumeZone::new(store.clone());
        let p = pod_with_pvc("p", "c");
        assert!(plug.filter(&p, &node_with_labels("a", &[
            ("topology.kubernetes.io/zone", "us-east-1a"),
        ]), &snap()).is_success());
    }

    #[test]
    fn volume_zone_rejects_wrong_zone() {
        let store = Arc::new(VolumeStore::new());
        store.add_pv(make_zoned_pv("pv0", "us-east-1a"));
        let mut c = pvc("c", None); c.volume_name = Some("pv0".into());
        store.add_pvc(c);
        let plug = VolumeZone::new(store.clone());
        let p = pod_with_pvc("p", "c");
        assert!(plug.filter(&p, &node_with_labels("a", &[
            ("topology.kubernetes.io/zone", "eu-west-1"),
        ]), &snap()).is_rejected());
    }

    #[test]
    fn volume_zone_rejects_node_missing_zone_label() {
        let store = Arc::new(VolumeStore::new());
        store.add_pv(make_zoned_pv("pv0", "us-east-1a"));
        let mut c = pvc("c", None); c.volume_name = Some("pv0".into());
        store.add_pvc(c);
        let plug = VolumeZone::new(store.clone());
        let p = pod_with_pvc("p", "c");
        assert!(plug.filter(&p, &node_with_labels("a", &[]), &snap()).is_rejected());
    }

    #[test]
    fn volume_zone_skips_unbound_pvc() {
        let store = Arc::new(VolumeStore::new());
        store.add_pvc(pvc("c", None)); // no volume_name
        let plug = VolumeZone::new(store.clone());
        let p = pod_with_pvc("p", "c");
        // Nothing to enforce yet.
        assert!(plug.filter(&p, &node_with_labels("a", &[]), &snap()).is_success());
    }

    #[test]
    fn volume_zone_handles_legacy_failure_domain_label() {
        let store = Arc::new(VolumeStore::new());
        let mut pv = PersistentVolume::default();
        pv.name = "pv0".into();
        pv.node_affinity.insert(
            "failure-domain.beta.kubernetes.io/zone".into(),
            vec!["us-west-2c".into()],
        );
        store.add_pv(pv);
        let mut c = pvc("c", None); c.volume_name = Some("pv0".into());
        store.add_pvc(c);
        let plug = VolumeZone::new(store.clone());
        let p = pod_with_pvc("p", "c");
        // Right legacy label.
        assert!(plug.filter(&p, &node_with_labels("a", &[
            ("failure-domain.beta.kubernetes.io/zone", "us-west-2c"),
        ]), &snap()).is_success());
    }

    #[test]
    fn volume_zone_score_returns_max_for_passing_node() {
        let store = Arc::new(VolumeStore::new());
        let plug = VolumeZone::new(store);
        let p = Pod::new("t", "ns", "p");
        assert_eq!(plug.score(&p, &node_with_labels("a", &[]), &snap()), MAX_NODE_SCORE);
    }

    // ── StorageClass / PV bookkeeping ─────────────────────────────────────

    #[test]
    fn store_round_trips_pv_pvc_storage_class() {
        let store = VolumeStore::new();
        store.add_pv(make_zoned_pv("pv0", "z1"));
        store.add_pvc(pvc("c", Some("fast")));
        store.add_storage_class(StorageClass {
            name: "fast".into(),
            provisioner: "test".into(),
            volume_binding_mode: VolumeBindingMode::Immediate,
            allowed_topologies: vec![],
        });
        assert!(store.get_pv("pv0").is_some());
        assert!(store.get_pvc("ns", "c").is_some());
        assert!(store.get_storage_class("fast").is_some());
        assert!(store.get_storage_class("ghost").is_none());
    }

    #[test]
    fn binding_mode_default_immediate_when_class_missing() {
        // No SC → treated as no constraints (= WaitForFirstConsumer-ish).
        let store = Arc::new(VolumeStore::new());
        store.add_pvc(pvc("c", None));
        let plug = VolumeBinding::new(store.clone());
        let p = pod_with_pvc("p", "c");
        assert!(plug.filter(&p, &node_with_labels("a", &[]), &snap()).is_success());
    }
}
