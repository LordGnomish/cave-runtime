//! DaemonSet controller — one pod per (eligible) node.
//!
//! Upstream: [`pkg/controller/daemon`]. The full controller computes the
//! schedulability of each node, respects taints/tolerations, and runs a
//! rolling update similar to Deployment.

use crate::types::{Cite, ControllerError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSetSpec {
    pub name: String,
    pub namespace: String,
    /// Optional node selector. Empty = match every node.
    pub node_selector: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    pub labels: Vec<(String, String)>,
    pub schedulable: bool,
    pub running_ds_pod: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonSetStatus {
    pub desired_number_scheduled: u32,
    pub current_number_scheduled: u32,
    pub number_ready: u32,
}

/// Returns true if `node` matches the DaemonSet's selector and is schedulable.
/// Mirrors `nodeShouldRunDaemonPod` in `pkg/controller/daemon/daemon_controller.go`.
pub fn node_should_run(spec: &DaemonSetSpec, node: &NodeView) -> bool {
    if !node.schedulable {
        return false;
    }
    spec.node_selector.iter().all(|(k, v)| {
        node.labels.iter().any(|(nk, nv)| nk == k && nv == v)
    })
}

/// Mirrors `manage` in `pkg/controller/daemon/daemon_controller.go`.
pub fn reconcile(
    spec: &DaemonSetSpec,
    nodes: &[NodeView],
    _tenant: &TenantId,
) -> Result<Reconcile, ControllerError> {
    let mut creates: u32 = 0;
    let mut deletes: u32 = 0;
    for n in nodes {
        let want = node_should_run(spec, n);
        match (want, n.running_ds_pod) {
            (true, false) => creates += 1,
            (false, true) => deletes += 1,
            _ => {}
        }
    }
    if creates == 0 && deletes == 0 {
        Ok(Reconcile::NoOp)
    } else if creates >= deletes {
        Ok(Reconcile::Create(creates))
    } else {
        Ok(Reconcile::Delete(deletes))
    }
}

/// Stub: surge-based rolling update. Not implemented.
pub fn rolling_update(_spec: &DaemonSetSpec) -> Result<Reconcile, ControllerError> {
    unimplemented!("DaemonSet RollingUpdate — see pkg/controller/daemon/update.go")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::new("pkg/controller/daemon/daemon_controller.go", "DaemonSetsController");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn ds(selector: Vec<(&str, &str)>) -> DaemonSetSpec {
        DaemonSetSpec {
            name: "node-exporter".into(),
            namespace: "monitoring".into(),
            node_selector: selector
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn node(name: &str, labels: &[(&str, &str)], schedulable: bool, has_pod: bool) -> NodeView {
        NodeView {
            name: name.into(),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            schedulable,
            running_ds_pod: has_pod,
        }
    }

    #[test]
    fn matches_every_schedulable_node_with_no_selector() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "nodeShouldRunDaemonPod",
            "tenant-ds-no-selector"
        );
        let s = ds(vec![]);
        let nodes = vec![
            node("a", &[], true, false),
            node("b", &[], false, false),
        ];
        assert!(node_should_run(&s, &nodes[0]));
        assert!(!node_should_run(&s, &nodes[1]));
        assert_eq!(reconcile(&s, &nodes, &tenant).unwrap(), Reconcile::Create(1));
    }

    #[test]
    fn selector_filters_out_unlabeled_nodes() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "nodeShouldRunDaemonPod",
            "tenant-ds-selector"
        );
        let s = ds(vec![("role", "edge")]);
        let nodes = vec![
            node("edge-1", &[("role", "edge")], true, false),
            node("core-1", &[("role", "core")], true, false),
        ];
        assert_eq!(reconcile(&s, &nodes, &tenant).unwrap(), Reconcile::Create(1));
    }

    #[test]
    fn deletes_pod_when_node_no_longer_eligible() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "manage",
            "tenant-ds-evict"
        );
        let s = ds(vec![("role", "edge")]);
        let nodes = vec![node("former-edge", &[("role", "core")], true, true)];
        assert_eq!(reconcile(&s, &nodes, &tenant).unwrap(), Reconcile::Delete(1));
    }

    #[test]
    fn no_op_when_every_node_already_correct() {
        let (_cite, tenant) = test_ctx!(
            "pkg/controller/daemon/daemon_controller.go",
            "manage",
            "tenant-ds-noop"
        );
        let s = ds(vec![]);
        let nodes = vec![node("a", &[], true, true), node("b", &[], true, true)];
        assert_eq!(reconcile(&s, &nodes, &tenant).unwrap(), Reconcile::NoOp);
    }
}
