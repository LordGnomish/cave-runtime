//! Cloud node controller.
//!
//! Mirrors `cloud-provider/controllers/node/node_controller.go`. On node
//! join, the controller:
//!
//! 1. Sets `node.spec.providerID`.
//! 2. Adds standard topology labels (`topology.kubernetes.io/zone`,
//!    `topology.kubernetes.io/region`, `node.kubernetes.io/instance-type`).
//! 3. Removes the
//!    `node.cloudprovider.kubernetes.io/uninitialized:NoSchedule`
//!    *initializer taint* once the cloud has finished annotating the node.

use crate::types::{Cite, CloudError, Reconcile, TenantId};
use serde::{Deserialize, Serialize};

/// Standard label keys (verbatim from upstream).
pub const LABEL_ZONE: &str = "topology.kubernetes.io/zone";
pub const LABEL_REGION: &str = "topology.kubernetes.io/region";
pub const LABEL_INSTANCE_TYPE: &str = "node.kubernetes.io/instance-type";

/// Standard initializer taint key.
pub const INITIALIZER_TAINT_KEY: &str = "node.cloudprovider.kubernetes.io/uninitialized";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub name: String,
    pub provider_id: Option<String>,
    pub zone: Option<String>,
    pub region: Option<String>,
    pub instance_type: Option<String>,
    pub initializer_taint_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudFacts {
    pub provider_id: String,
    pub zone: String,
    pub region: String,
    pub instance_type: String,
}

/// True iff every standard label has been written.
pub fn is_initialised(node: &NodeView) -> bool {
    node.provider_id.is_some()
        && node.zone.is_some()
        && node.region.is_some()
        && node.instance_type.is_some()
}

/// Mirrors `syncNode` in upstream.
pub fn reconcile(
    node: &NodeView,
    facts: &CloudFacts,
    _tenant: &TenantId,
) -> Result<Reconcile, CloudError> {
    if !is_initialised(node) {
        // Count of fields that still need writing.
        let mut writes: u32 = 0;
        if node.provider_id.as_deref() != Some(facts.provider_id.as_str()) {
            writes += 1;
        }
        if node.zone.as_deref() != Some(facts.zone.as_str()) {
            writes += 1;
        }
        if node.region.as_deref() != Some(facts.region.as_str()) {
            writes += 1;
        }
        if node.instance_type.as_deref() != Some(facts.instance_type.as_str()) {
            writes += 1;
        }
        return Ok(Reconcile::Annotate(writes));
    }
    if node.initializer_taint_present {
        // All facts are written; drop the taint to admit pods.
        return Ok(Reconcile::Untaint(1));
    }
    Ok(Reconcile::NoOp)
}

/// Stub: shutdown handling — when a cloud reports the node as `terminated`,
/// the controller deletes the Node object. Not implemented in this scaffold.
pub fn handle_shutdown(_node: &NodeView) -> Result<Reconcile, CloudError> {
    unimplemented!("Node shutdown — see InstanceShutdownByProviderID in upstream")
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::k8s(
    "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
    "CloudNodeController",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_ctx;

    fn fresh_node(name: &str) -> NodeView {
        NodeView {
            name: name.into(),
            provider_id: None,
            zone: None,
            region: None,
            instance_type: None,
            initializer_taint_present: true,
        }
    }

    fn facts() -> CloudFacts {
        CloudFacts {
            provider_id: "hcloud://1234".into(),
            zone: "fsn1-dc14".into(),
            region: "fsn1".into(),
            instance_type: "cpx21".into(),
        }
    }

    #[test]
    fn fresh_node_needs_four_annotations() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-fresh"
        );
        let n = fresh_node("worker-1");
        assert!(!is_initialised(&n));
        assert_eq!(reconcile(&n, &facts(), &tenant).unwrap(), Reconcile::Annotate(4));
    }

    #[test]
    fn partially_annotated_node_writes_only_diff() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "updateNodeAddress",
            "tenant-node-partial"
        );
        let mut n = fresh_node("worker-2");
        let f = facts();
        n.provider_id = Some(f.provider_id.clone());
        n.region = Some(f.region.clone());
        // Two fields still missing → 2 writes.
        assert_eq!(reconcile(&n, &f, &tenant).unwrap(), Reconcile::Annotate(2));
    }

    #[test]
    fn fully_initialised_node_drops_initializer_taint() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "ensureNodeExistsByProviderID",
            "tenant-node-untaint"
        );
        let f = facts();
        let n = NodeView {
            name: "worker-3".into(),
            provider_id: Some(f.provider_id.clone()),
            zone: Some(f.zone.clone()),
            region: Some(f.region.clone()),
            instance_type: Some(f.instance_type.clone()),
            initializer_taint_present: true,
        };
        assert!(is_initialised(&n));
        assert_eq!(reconcile(&n, &f, &tenant).unwrap(), Reconcile::Untaint(1));
    }

    #[test]
    fn fully_initialised_untainted_node_is_a_no_op() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go",
            "syncNode",
            "tenant-node-noop"
        );
        let f = facts();
        let n = NodeView {
            name: "worker-4".into(),
            provider_id: Some(f.provider_id.clone()),
            zone: Some(f.zone.clone()),
            region: Some(f.region.clone()),
            instance_type: Some(f.instance_type.clone()),
            initializer_taint_present: false,
        };
        assert_eq!(reconcile(&n, &f, &tenant).unwrap(), Reconcile::NoOp);
    }

    #[test]
    fn label_constants_match_upstream_keys() {
        let (_cite, tenant) = test_ctx!(
            "staging/src/k8s.io/api/core/v1/well_known_labels.go",
            "LabelTopologyZone",
            "tenant-node-labels"
        );
        let _ = tenant;
        assert_eq!(LABEL_ZONE, "topology.kubernetes.io/zone");
        assert_eq!(LABEL_REGION, "topology.kubernetes.io/region");
        assert_eq!(LABEL_INSTANCE_TYPE, "node.kubernetes.io/instance-type");
        assert_eq!(INITIALIZER_TAINT_KEY, "node.cloudprovider.kubernetes.io/uninitialized");
    }
}
