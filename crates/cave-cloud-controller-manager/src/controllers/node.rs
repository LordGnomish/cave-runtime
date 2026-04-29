//! Cloud node controller — initializes a Node with provider metadata.
//!
//! Upstream: staging/src/k8s.io/cloud-provider/controllers/node/node_controller.go::InitializeNode

use crate::models::CloudNode;
use crate::provider::CloudProvider;

/// Reconcile a Node by populating its provider-supplied fields when missing.
///
/// Returns the same node observation with `initialized = true` once provider
/// metadata has been applied; returns the node unchanged if metadata is not
/// available (the controller will retry on the next reconcile).
pub fn reconcile_node(mut node: CloudNode, provider: &dyn CloudProvider) -> CloudNode {
    if node.initialized {
        return node;
    }
    if let Some(meta) = provider.instance_metadata(&node.name) {
        node.provider_id = Some(meta.provider_id);
        node.instance_type = Some(meta.instance_type);
        node.zone = Some(meta.zone);
        node.addresses = meta.addresses;
        node.initialized = true;
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::FakeProvider;

    fn provider() -> FakeProvider {
        let mut p = FakeProvider::new("fake");
        p.add_node("worker-1", "m5.large", "us-east-1a", "10.0.0.1");
        p
    }

    /// Port of `TestCloudNodeControllerInitialize` — a fresh Node with a known
    /// instance is populated with provider metadata and marked initialized.
    /// An unknown node remains uninitialized so the controller can retry later.
    #[test]
    fn test_cloud_node_controller_initialize() {
        let p = provider();

        let known = reconcile_node(CloudNode::new("worker-1"), &p);
        assert!(known.initialized);
        assert_eq!(known.instance_type.as_deref(), Some("m5.large"));
        assert_eq!(known.zone.as_deref(), Some("us-east-1a"));
        assert_eq!(known.addresses.len(), 1);

        let unknown = reconcile_node(CloudNode::new("ghost"), &p);
        assert!(!unknown.initialized);
        assert!(unknown.instance_type.is_none());
    }

    #[test]
    fn test_cloud_node_idempotent() {
        let p = provider();
        let once = reconcile_node(CloudNode::new("worker-1"), &p);
        let zone_first = once.zone.clone();
        let twice = reconcile_node(once, &p);
        assert!(twice.initialized);
        assert_eq!(twice.zone, zone_first);
    }
}
