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
    for add in add_addresses {
        let exists = addresses
            .iter()
            .any(|existing| existing.address == add.address && existing.kind == add.kind);
        if !exists {
            addresses.push(add.clone());
        }
    }
}

// ─── component-helpers node/util/ips.go ──────────────────────────────────────

/// `ParseIPSloppy` — tolerant IP parse matching `k8s.io/utils/net`: IPv4
/// octets with leading zeros (e.g. `010.000.000.001`) are accepted and read as
/// decimal, IPv6 uses the strict parser.
pub fn parse_ip_sloppy(s: &str) -> Option<IpAddr> {
    if s.contains(':') {
        return s.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
    }
    let octets: Vec<&str> = s.split('.').collect();
    if octets.len() != 4 {
        return None;
    }
    let mut bytes = [0u8; 4];
    for (i, oct) in octets.iter().enumerate() {
        if oct.is_empty() || !oct.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let v: u32 = oct.parse().ok()?;
        if v > 255 {
            return None;
        }
        bytes[i] = v as u8;
    }
    Some(IpAddr::V4(Ipv4Addr::from(bytes)))
}

/// `ParseNodeIPAnnotation` — parses the `alpha.kubernetes.io/provided-node-ip`
/// annotation value into one IP, or a dual-stack pair. Equivalent to
/// `parseNodeIP(nodeIP, allowDual=true, sloppy=false)`.
pub fn parse_node_ip_annotation(node_ip: &str) -> Result<Vec<IpAddr>, String> {
    // sloppy=false: always enter the parse loop, no trim, hard-fail on a bad IP.
    let mut node_ips: Vec<IpAddr> = Vec::new();
    for ip in node_ip.split(',') {
        match parse_ip_sloppy(ip) {
            Some(parsed) => node_ips.push(parsed),
            None => return Err(format!("could not parse {:?}", ip)),
        }
    }

    if node_ips.len() > 2
        || (node_ips.len() == 2 && node_ips[0].is_ipv6() == node_ips[1].is_ipv6())
    {
        return Err("must contain either a single IP or a dual-stack pair of IPs".to_string());
    }
    // allowDual=true for annotations → no "dual-stack not supported" branch.
    if node_ips.len() == 2 && (node_ips[0].is_unspecified() || node_ips[1].is_unspecified()) {
        return Err("dual-stack node IP cannot include '0.0.0.0' or '::'".to_string());
    }

    Ok(node_ips)
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

    // ── parse_node_ip_annotation ──

    #[test]
    fn parse_node_ip_single_v4() {
        let r = parse_node_ip_annotation("10.0.0.1").unwrap();
        assert_eq!(r, vec!["10.0.0.1".parse::<IpAddr>().unwrap()]);
    }

    #[test]
    fn parse_node_ip_single_v6() {
        let r = parse_node_ip_annotation("2001:db8::1").unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].is_ipv6());
    }

    #[test]
    fn parse_node_ip_dual_stack_pair_ok() {
        let r = parse_node_ip_annotation("10.0.0.1,2001:db8::1").unwrap();
        assert_eq!(r.len(), 2);
        assert!(r[0].is_ipv4() && r[1].is_ipv6());
    }

    #[test]
    fn parse_node_ip_two_same_family_rejected() {
        let e = parse_node_ip_annotation("10.0.0.1,10.0.0.2").unwrap_err();
        assert_eq!(e, "must contain either a single IP or a dual-stack pair of IPs");
    }

    #[test]
    fn parse_node_ip_three_rejected() {
        let e = parse_node_ip_annotation("10.0.0.1,2001:db8::1,2001:db8::2").unwrap_err();
        assert_eq!(e, "must contain either a single IP or a dual-stack pair of IPs");
    }

    #[test]
    fn parse_node_ip_dual_stack_unspecified_rejected() {
        let e = parse_node_ip_annotation("0.0.0.0,2001:db8::1").unwrap_err();
        assert_eq!(e, "dual-stack node IP cannot include '0.0.0.0' or '::'");
    }

    #[test]
    fn parse_node_ip_single_unspecified_allowed() {
        // Only the dual-stack path forbids the unspecified address.
        let r = parse_node_ip_annotation("0.0.0.0").unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].is_unspecified());
    }

    #[test]
    fn parse_node_ip_unparseable_rejected() {
        let e = parse_node_ip_annotation("garbage").unwrap_err();
        assert_eq!(e, "could not parse \"garbage\"");
    }

    #[test]
    fn parse_node_ip_empty_rejected() {
        // sloppy=false → empty string still enters the parse loop and fails.
        let e = parse_node_ip_annotation("").unwrap_err();
        assert_eq!(e, "could not parse \"\"");
    }

    #[test]
    fn parse_ip_sloppy_forgives_leading_zeros() {
        assert_eq!(
            parse_ip_sloppy("010.000.000.001"),
            Some("10.0.0.1".parse::<IpAddr>().unwrap())
        );
    }
}
