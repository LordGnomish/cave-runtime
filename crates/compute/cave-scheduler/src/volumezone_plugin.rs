// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VolumeZone — reject nodes whose zone label is not allowed by any of the
//! pod's bound PVs.
//!
//! Cite: kubernetes/kubernetes v1.36.0
//!   pkg/scheduler/framework/plugins/volumezone/volume_zone.go
//!
//! When a Pod references a PVC that's bound to a zone-restricted
//! `PersistentVolume` (e.g. EBS in `us-east-1a`), the scheduler must place
//! the Pod on a node sitting in that same zone — otherwise the kubelet on
//! the chosen node will fail to attach the volume.
//!
//! The check honours three zone-related labels (upstream's
//! `topologyLabels`):
//!
//! - `topology.kubernetes.io/zone`
//! - `topology.kubernetes.io/region`
//! - `failure-domain.beta.kubernetes.io/zone` (legacy)
//!
//! For each bound PV that has a matching `node_affinity` entry, the node's
//! label value must appear in the PV's allowed list. PVs with no
//! `node_affinity` for these keys are unconstrained.
//!
//! Pods with no PVCs (or only unbound PVCs) pass — VolumeZone only enforces
//! once the binder has nailed down the actual PV.
//!
//! Wired into the framework via the `volumezone_filter_plugin` constructor
//! so callers can `framework.with_filter(volumezone_filter_plugin(store))`.

use crate::framework::{ClusterSnapshot, FilterPlugin, Pod, Status, VolumeKind};
use crate::models::Node;
use crate::volume::VolumeStore;
use std::sync::Arc;

/// Zone-related label keys upstream's volumezone honours. Each entry is
/// matched against the node's labels and the PV's `node_affinity` allowed
/// values.
pub const ZONE_LABELS: &[&str] = &[
    "topology.kubernetes.io/zone",
    "topology.kubernetes.io/region",
    "failure-domain.beta.kubernetes.io/zone",
];

/// VolumeZone Filter plugin — wraps a [`VolumeStore`] reference so it can
/// look up bound PVs at scheduling time.
pub struct VolumeZone {
    pub store: Arc<VolumeStore>,
}

impl VolumeZone {
    pub fn new(store: Arc<VolumeStore>) -> Self {
        Self { store }
    }
}

impl FilterPlugin for VolumeZone {
    fn name(&self) -> &str {
        "VolumeZone"
    }

    fn filter(&self, pod: &Pod, node: &Node, _snap: &ClusterSnapshot) -> Status {
        for vol in &pod.spec.volumes {
            let claim_name = match &vol.kind {
                VolumeKind::PersistentVolumeClaim { claim_name, .. } => claim_name,
                _ => continue,
            };
            let Some(pvc) = self.store.get_pvc(&pod.namespace, claim_name) else {
                // PVC not registered → not yet visible, skip.
                continue;
            };
            let Some(pv_name) = pvc.volume_name.as_deref() else {
                // PVC unbound — VolumeBinding plugin handles WFC.
                continue;
            };
            let Some(pv) = self.store.get_pv(pv_name) else {
                // PV vanished from the store; treat as not yet visible.
                continue;
            };
            if pv.node_affinity.is_empty() {
                continue;
            }

            for label in ZONE_LABELS {
                let Some(allowed) = pv.node_affinity.get(*label) else {
                    continue;
                };
                let Some(have) = node.labels.get(*label) else {
                    // PV constrains a zone, but the node has no value for that
                    // key → reject (upstream behaviour: the absence of a node
                    // label means the node cannot satisfy a zone term).
                    return Status::unschedulable(
                        "VolumeZone",
                        format!(
                            "node has no value for {label}; PV {pv_name} requires one of {allowed:?}"
                        ),
                    );
                };
                if !allowed.iter().any(|v| v == have) {
                    return Status::unschedulable(
                        "VolumeZone",
                        format!(
                            "node {label}={have} not in PV {pv_name} allowed zones {allowed:?}"
                        ),
                    );
                }
            }
        }
        Status::success("VolumeZone")
    }
}

/// Builder constructor — returns a boxed plugin for
/// `Framework::with_filter(volumezone_filter_plugin(store))`.
pub fn volumezone_filter_plugin(store: Arc<VolumeStore>) -> Box<dyn FilterPlugin> {
    Box::new(VolumeZone::new(store))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::{Pod, VolumeKind, VolumeSpec};
    use crate::models::{Node as ModelNode, NodeStatus, ResourceCapacity};
    use crate::volume::{PersistentVolume, PersistentVolumeClaim, VolumeStore};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn node_with_zone(name: &str, zone: &str) -> ModelNode {
        let mut labels = HashMap::new();
        labels.insert("topology.kubernetes.io/zone".to_string(), zone.to_string());
        ModelNode {
            name: name.into(),
            uid: Uuid::new_v4(),
            status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity::default(),
            allocated: ResourceCapacity::default(),
            labels,
            taints: vec![],
            conditions: vec![],
            registered_at: Utc::now(),
            last_heartbeat: Utc::now(),
        }
    }

    fn pod_with_pvc(ns: &str, name: &str, claim: &str) -> Pod {
        let mut p = Pod::new("t", ns, name);
        p.spec.volumes.push(VolumeSpec {
            name: "data".into(),
            kind: VolumeKind::PersistentVolumeClaim {
                claim_name: claim.into(),
                bound_node: None,
            },
        });
        p
    }

    #[test]
    fn empty_pod_with_no_pvcs_passes() {
        let store = Arc::new(VolumeStore::new());
        let plug = VolumeZone::new(store);
        let pod = Pod::new("t", "ns", "x");
        let node = node_with_zone("n1", "us-east-1a");
        let snap = ClusterSnapshot::default();
        assert!(plug.filter(&pod, &node, &snap).is_success());
    }

    #[test]
    fn unbound_pvc_skips_check() {
        let store = Arc::new(VolumeStore::new());
        // PVC exists but is unbound (volume_name = None) → skip.
        store.add_pvc(PersistentVolumeClaim {
            name: "data-0".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            storage_class: None,
            access_modes: Default::default(),
            requested_bytes: 0,
            volume_name: None,
            bound_node: None,
        });

        let plug = VolumeZone::new(store);
        let pod = pod_with_pvc("ns", "x", "data-0");
        let node = node_with_zone("n1", "us-east-1a");
        let snap = ClusterSnapshot::default();
        assert!(plug.filter(&pod, &node, &snap).is_success());
    }

    #[test]
    fn bound_pv_single_zone_matches_node() {
        let store = Arc::new(VolumeStore::new());
        let mut pv = PersistentVolume {
            name: "pv-data-0".into(),
            ..Default::default()
        };
        pv.node_affinity.insert(
            "topology.kubernetes.io/zone".into(),
            vec!["us-east-1a".into()],
        );
        store.add_pv(pv);

        let mut pvc = PersistentVolumeClaim {
            name: "data-0".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            ..Default::default()
        };
        pvc.volume_name = Some("pv-data-0".into());
        store.add_pvc(pvc);

        let plug = VolumeZone::new(store);
        let pod = pod_with_pvc("ns", "x", "data-0");
        let node = node_with_zone("n1", "us-east-1a");
        let snap = ClusterSnapshot::default();
        assert!(plug.filter(&pod, &node, &snap).is_success());
    }

    #[test]
    fn multi_zone_pv_matches_any_allowed_zone() {
        let store = Arc::new(VolumeStore::new());
        let mut pv = PersistentVolume {
            name: "pv-data-0".into(),
            ..Default::default()
        };
        pv.node_affinity.insert(
            "topology.kubernetes.io/zone".into(),
            vec!["us-east-1a".into(), "us-east-1c".into()],
        );
        store.add_pv(pv);

        let mut pvc = PersistentVolumeClaim {
            name: "data-0".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            ..Default::default()
        };
        pvc.volume_name = Some("pv-data-0".into());
        store.add_pvc(pvc);

        let plug = VolumeZone::new(store);
        let pod = pod_with_pvc("ns", "x", "data-0");
        let node = node_with_zone("n1", "us-east-1c");
        let snap = ClusterSnapshot::default();
        assert!(plug.filter(&pod, &node, &snap).is_success());
    }

    #[test]
    fn mismatched_zone_rejects_node() {
        let store = Arc::new(VolumeStore::new());
        let mut pv = PersistentVolume {
            name: "pv-data-0".into(),
            ..Default::default()
        };
        pv.node_affinity.insert(
            "topology.kubernetes.io/zone".into(),
            vec!["us-east-1a".into()],
        );
        store.add_pv(pv);

        let mut pvc = PersistentVolumeClaim {
            name: "data-0".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            ..Default::default()
        };
        pvc.volume_name = Some("pv-data-0".into());
        store.add_pvc(pvc);

        let plug = VolumeZone::new(store);
        let pod = pod_with_pvc("ns", "x", "data-0");
        let node = node_with_zone("n1", "eu-west-1");
        let snap = ClusterSnapshot::default();
        let status = plug.filter(&pod, &node, &snap);
        assert!(status.is_rejected(), "expected rejection, got {status:?}");
        assert_eq!(status.plugin, "VolumeZone");
    }

    #[test]
    fn node_missing_zone_label_rejected_when_pv_requires_one() {
        let store = Arc::new(VolumeStore::new());
        let mut pv = PersistentVolume {
            name: "pv-data-0".into(),
            ..Default::default()
        };
        pv.node_affinity.insert(
            "topology.kubernetes.io/zone".into(),
            vec!["us-east-1a".into()],
        );
        store.add_pv(pv);

        let mut pvc = PersistentVolumeClaim {
            name: "data-0".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            ..Default::default()
        };
        pvc.volume_name = Some("pv-data-0".into());
        store.add_pvc(pvc);

        let plug = VolumeZone::new(store);
        let pod = pod_with_pvc("ns", "x", "data-0");

        // Build a node with no zone label.
        let node = ModelNode {
            name: "no-zone".into(),
            uid: Uuid::new_v4(),
            status: NodeStatus::Ready,
            capacity: ResourceCapacity::default(),
            allocatable: ResourceCapacity::default(),
            allocated: ResourceCapacity::default(),
            labels: HashMap::new(),
            taints: vec![],
            conditions: vec![],
            registered_at: Utc::now(),
            last_heartbeat: Utc::now(),
        };
        let snap = ClusterSnapshot::default();
        assert!(plug.filter(&pod, &node, &snap).is_rejected());
    }

    #[test]
    fn pv_with_no_zone_affinity_is_unconstrained() {
        let store = Arc::new(VolumeStore::new());
        // PV exists, bound, but no node_affinity → any node is fine.
        let pv = PersistentVolume {
            name: "pv-data-0".into(),
            ..Default::default()
        };
        store.add_pv(pv);

        let mut pvc = PersistentVolumeClaim {
            name: "data-0".into(),
            namespace: "ns".into(),
            tenant_id: "t".into(),
            ..Default::default()
        };
        pvc.volume_name = Some("pv-data-0".into());
        store.add_pvc(pvc);

        let plug = VolumeZone::new(store);
        let pod = pod_with_pvc("ns", "x", "data-0");
        let node = node_with_zone("n1", "anywhere");
        let snap = ClusterSnapshot::default();
        assert!(plug.filter(&pod, &node, &snap).is_success());
    }

    #[test]
    fn builder_returns_boxed_filter_plugin() {
        let store = Arc::new(VolumeStore::new());
        let p: Box<dyn FilterPlugin> = volumezone_filter_plugin(store);
        assert_eq!(p.name(), "VolumeZone");
    }
}
