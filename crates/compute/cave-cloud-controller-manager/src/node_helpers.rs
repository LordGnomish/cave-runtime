//! 1:1 port of `staging/src/k8s.io/cloud-provider/node/helpers/` — the broader
//! node-helpers pack that was previously only *partially* represented by the
//! ad-hoc address utilities in [`crate::node_controller`].
//!
//! Upstream files mapped here:
//!   * `node/helpers/address.go`  → [`add_to_node_addresses`], [`get_node_addresses_from_node_ip`]
//!   * `node/helpers/taints.go`   → [`add_or_update_taint`], [`remove_taint`], [`taint_exists`]
//!   * `k8s.io/component-helpers/node/util/ips.go` → [`parse_node_ip_annotation`]
//!   * `k8s.io/api/core/v1.Taint`  → [`Taint`] (`MatchTaint`/`ToString`)
//!
//! Behaviour is matched against kubernetes/cloud-provider @ v1.36 and
//! kubernetes/component-helpers. The functions here are pure (no clientset);
//! the API-issuing wrappers (`AddOrUpdateTaintOnNode`, `RemoveTaintOffNode`)
//! reduce, in this in-process runtime, to their pure inner helpers.

use crate::node_controller::{NodeAddress, NodeAddressType};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

// ─── address.go ──────────────────────────────────────────────────────────────

/// `AddToNodeAddresses` — append `add_addresses` onto `addresses`, skipping any
/// entry whose `(type, address)` pair already exists. Mirrors upstream exactly:
/// dedup key is the full `(Type, Address)` tuple, order is preserved.
pub fn add_to_node_addresses(addresses: &mut Vec<NodeAddress>, add_addresses: &[NodeAddress]) {
    let _ = (addresses, add_addresses);
    unimplemented!("RED: add_to_node_addresses")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(kind: NodeAddressType, a: &str) -> NodeAddress {
        NodeAddress::new(kind, a)
    }

    #[test]
    fn add_to_node_addresses_appends_new_entries() {
        let mut addrs = vec![ip(NodeAddressType::InternalIP, "10.0.0.1")];
        add_to_node_addresses(
            &mut addrs,
            &[
                ip(NodeAddressType::ExternalIP, "1.2.3.4"),
                ip(NodeAddressType::Hostname, "node-a"),
            ],
        );
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[1], ip(NodeAddressType::ExternalIP, "1.2.3.4"));
        assert_eq!(addrs[2], ip(NodeAddressType::Hostname, "node-a"));
    }

    #[test]
    fn add_to_node_addresses_skips_exact_duplicates() {
        let mut addrs = vec![ip(NodeAddressType::InternalIP, "10.0.0.1")];
        add_to_node_addresses(&mut addrs, &[ip(NodeAddressType::InternalIP, "10.0.0.1")]);
        assert_eq!(addrs.len(), 1);
    }

    #[test]
    fn add_to_node_addresses_same_address_different_type_is_not_duplicate() {
        // (Type, Address) is the dedup key — same address under a different
        // type must still be appended.
        let mut addrs = vec![ip(NodeAddressType::InternalIP, "10.0.0.1")];
        add_to_node_addresses(&mut addrs, &[ip(NodeAddressType::ExternalIP, "10.0.0.1")]);
        assert_eq!(addrs.len(), 2);
    }

    #[test]
    fn add_to_node_addresses_dedups_within_the_added_batch() {
        let mut addrs: Vec<NodeAddress> = vec![];
        add_to_node_addresses(
            &mut addrs,
            &[
                ip(NodeAddressType::InternalIP, "10.0.0.1"),
                ip(NodeAddressType::InternalIP, "10.0.0.1"),
            ],
        );
        assert_eq!(addrs.len(), 1);
    }
}
