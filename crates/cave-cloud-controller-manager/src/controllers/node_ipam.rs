//! Node IPAM controller — allocates a per-node Pod CIDR slice from the
//! cluster pool.
//!
//! Upstream: staging/src/k8s.io/cloud-provider/controllers/nodeipam/node_ipam_controller.go::AllocateOrOccupyCIDR

use crate::models::CloudNode;

/// IPAM allocator state — tracks how many /24 slices have been handed out
/// from the configured cluster_cidr.
pub struct IpamAllocator {
    pub cluster_cidr_prefix: String, // e.g. "10.244"
    pub next_slice: u8,
}

impl IpamAllocator {
    pub fn new(cluster_cidr_prefix: &str) -> Self {
        Self {
            cluster_cidr_prefix: cluster_cidr_prefix.to_string(),
            next_slice: 0,
        }
    }
}

/// Reconcile a Node's Pod CIDR. If the node already has one, leave it; if
/// not, allocate the next /24 slice from the allocator.
pub fn reconcile_node_ipam(mut node: CloudNode, allocator: &mut IpamAllocator) -> CloudNode {
    if node.pod_cidr.is_some() {
        return node;
    }
    let cidr = format!("{}.{}.0/24", allocator.cluster_cidr_prefix, allocator.next_slice);
    allocator.next_slice = allocator.next_slice.saturating_add(1);
    node.pod_cidr = Some(cidr);
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `TestNodeIpamAllocateCidr` — successive nodes receive
    /// non-overlapping /24 slices in stable order.
    #[test]
    fn test_node_ipam_allocate_cidr() {
        let mut a = IpamAllocator::new("10.244");
        let n1 = reconcile_node_ipam(CloudNode::new("n1"), &mut a);
        let n2 = reconcile_node_ipam(CloudNode::new("n2"), &mut a);
        assert_eq!(n1.pod_cidr.as_deref(), Some("10.244.0.0/24"));
        assert_eq!(n2.pod_cidr.as_deref(), Some("10.244.1.0/24"));
    }

    /// Port of `TestNodeIpamPreservesExisting` — nodes that already carry a
    /// CIDR are left untouched and don't consume a new slice.
    #[test]
    fn test_node_ipam_preserves_existing() {
        let mut a = IpamAllocator::new("10.244");
        let mut existing = CloudNode::new("n1");
        existing.pod_cidr = Some("172.16.0.0/24".into());
        let after = reconcile_node_ipam(existing, &mut a);
        assert_eq!(after.pod_cidr.as_deref(), Some("172.16.0.0/24"));
        // allocator was not advanced
        assert_eq!(a.next_slice, 0);
    }
}
