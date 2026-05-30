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

/// `GetNodeAddressesFromNodeIP` — given the operator-provided node IP(s) and
/// the cloud-reported addresses, return the enforced address set: every cloud
/// address whose IP equals a provided node IP comes first (and pins its type),
/// then every remaining cloud address whose type was *not* pinned. Errors if a
/// provided IP matches none of the cloud addresses. Mirrors address.go.
pub fn get_node_addresses_from_node_ip(
    provided_node_ip: &str,
    cloud_node_addresses: &[NodeAddress],
) -> Result<Vec<NodeAddress>, String> {
    let node_ips = parse_node_ip_annotation(provided_node_ip)
        .map_err(|e| format!("failed to parse node IP {:?}: {}", provided_node_ip, e))?;

    let mut enforced: Vec<NodeAddress> = Vec::new();
    let mut pinned_types: Vec<NodeAddressType> = Vec::new();

    for node_ip in &node_ips {
        let mut matched = false;
        for addr in cloud_node_addresses {
            if parse_ip_sloppy(&addr.address) == Some(*node_ip) {
                enforced.push(addr.clone());
                if !pinned_types.contains(&addr.kind) {
                    pinned_types.push(addr.kind);
                }
                matched = true;
            }
        }
        if !matched {
            return Err(format!(
                "failed to get node address from cloud provider that matches ip: {}",
                node_ip
            ));
        }
    }

    for addr in cloud_node_addresses {
        if !pinned_types.contains(&addr.kind) {
            enforced.push(addr.clone());
        }
    }

    Ok(enforced)
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

// ─── taints.go + core/v1.Taint ───────────────────────────────────────────────

/// Mirror of `core/v1.Taint`. Identity (`MatchTaint`) is `(key, effect)`; the
/// `value` participates only in full equality.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taint {
    pub key: String,
    pub value: String,
    pub effect: String,
}

impl Taint {
    pub fn new(key: impl Into<String>, value: impl Into<String>, effect: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            effect: effect.into(),
        }
    }

    /// `Taint.MatchTaint` — equal iff `key` and `effect` match (value ignored).
    pub fn match_taint(&self, other: &Taint) -> bool {
        self.key == other.key && self.effect == other.effect
    }

    /// `Taint.ToString` — `key=value:effect`, collapsing empty value/effect.
    pub fn to_string(&self) -> String {
        if self.effect.is_empty() {
            if self.value.is_empty() {
                return self.key.clone();
            }
            return format!("{}={}:", self.key, self.value);
        }
        if self.value.is_empty() {
            return format!("{}:{}", self.key, self.effect);
        }
        format!("{}={}:{}", self.key, self.value, self.effect)
    }
}

/// `taintExists` — linear search by `MatchTaint`.
pub fn taint_exists(taints: &[Taint], taint: &Taint) -> bool {
    taints.iter().any(|t| t.match_taint(taint))
}

/// `addOrUpdateTaint` — returns the new taint list and whether the node changed.
/// If a `(key,effect)`-matching taint exists and is fully equal → unchanged
/// (`false`); if it matches but differs → replaced (`true`); if no match → the
/// taint is appended (`true`).
pub fn add_or_update_taint(taints: &[Taint], taint: &Taint) -> (Vec<Taint>, bool) {
    let mut new_taints: Vec<Taint> = Vec::new();
    let mut updated = false;
    for existing in taints {
        if taint.match_taint(existing) {
            if taint == existing {
                // Fully equal — node unchanged.
                return (taints.to_vec(), false);
            }
            new_taints.push(taint.clone());
            updated = true;
            continue;
        }
        new_taints.push(existing.clone());
    }
    if !updated {
        new_taints.push(taint.clone());
    }
    (new_taints, true)
}

/// `deleteTaint` — drop every taint matching `(key,effect)`; returns the list
/// and whether anything was deleted.
pub fn delete_taint(taints: &[Taint], taint_to_delete: &Taint) -> (Vec<Taint>, bool) {
    let mut new_taints: Vec<Taint> = Vec::new();
    let mut deleted = false;
    for t in taints {
        if taint_to_delete.match_taint(t) {
            deleted = true;
            continue;
        }
        new_taints.push(t.clone());
    }
    (new_taints, deleted)
}

/// `removeTaint` — no-op (`false`) when the list is empty or the taint is
/// absent; otherwise deletes it and reports `true`.
pub fn remove_taint(taints: &[Taint], taint: &Taint) -> (Vec<Taint>, bool) {
    if taints.is_empty() {
        return (taints.to_vec(), false);
    }
    if !taint_exists(taints, taint) {
        return (taints.to_vec(), false);
    }
    let (new_taints, _) = delete_taint(taints, taint);
    (new_taints, true)
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

    // ── get_node_addresses_from_node_ip ──

    #[test]
    fn enforce_node_ip_matched_first_then_other_types() {
        let cloud = vec![
            ip(NodeAddressType::InternalIP, "10.0.0.1"),
            ip(NodeAddressType::ExternalIP, "1.2.3.4"),
            ip(NodeAddressType::Hostname, "node-a"),
        ];
        let r = get_node_addresses_from_node_ip("10.0.0.1", &cloud).unwrap();
        assert_eq!(
            r,
            vec![
                ip(NodeAddressType::InternalIP, "10.0.0.1"),
                ip(NodeAddressType::ExternalIP, "1.2.3.4"),
                ip(NodeAddressType::Hostname, "node-a"),
            ]
        );
    }

    #[test]
    fn enforce_node_ip_pins_type_dropping_same_type_siblings() {
        // Matching one ExternalIP pins the ExternalIP type, so the *other*
        // ExternalIP is dropped (its type is already enforced).
        let cloud = vec![
            ip(NodeAddressType::InternalIP, "10.0.0.1"),
            ip(NodeAddressType::ExternalIP, "1.2.3.4"),
            ip(NodeAddressType::ExternalIP, "5.6.7.8"),
        ];
        let r = get_node_addresses_from_node_ip("5.6.7.8", &cloud).unwrap();
        assert_eq!(
            r,
            vec![
                ip(NodeAddressType::ExternalIP, "5.6.7.8"),
                ip(NodeAddressType::InternalIP, "10.0.0.1"),
            ]
        );
    }

    #[test]
    fn enforce_node_ip_no_match_errors() {
        let cloud = vec![ip(NodeAddressType::InternalIP, "10.0.0.1")];
        let e = get_node_addresses_from_node_ip("192.168.0.9", &cloud).unwrap_err();
        assert_eq!(
            e,
            "failed to get node address from cloud provider that matches ip: 192.168.0.9"
        );
    }

    #[test]
    fn enforce_node_ip_sloppy_address_equality() {
        // Cloud reports a leading-zero address; provided IP is canonical.
        let cloud = vec![ip(NodeAddressType::InternalIP, "010.000.000.001")];
        let r = get_node_addresses_from_node_ip("10.0.0.1", &cloud).unwrap();
        assert_eq!(r, vec![ip(NodeAddressType::InternalIP, "010.000.000.001")]);
    }

    #[test]
    fn enforce_node_ip_dual_stack_both_pinned() {
        let cloud = vec![
            ip(NodeAddressType::InternalIP, "10.0.0.1"),
            ip(NodeAddressType::InternalIP, "2001:db8::1"),
            ip(NodeAddressType::Hostname, "node-a"),
        ];
        let r = get_node_addresses_from_node_ip("10.0.0.1,2001:db8::1", &cloud).unwrap();
        assert_eq!(
            r,
            vec![
                ip(NodeAddressType::InternalIP, "10.0.0.1"),
                ip(NodeAddressType::InternalIP, "2001:db8::1"),
                ip(NodeAddressType::Hostname, "node-a"),
            ]
        );
    }

    #[test]
    fn enforce_node_ip_parse_error_propagates() {
        let cloud = vec![ip(NodeAddressType::InternalIP, "10.0.0.1")];
        let e = get_node_addresses_from_node_ip("garbage", &cloud).unwrap_err();
        assert_eq!(e, "failed to parse node IP \"garbage\": could not parse \"garbage\"");
    }

    // ── taints ──

    fn taint(k: &str, v: &str, e: &str) -> Taint {
        Taint::new(k, v, e)
    }

    #[test]
    fn match_taint_is_key_and_effect_only() {
        let a = taint("k1", "v1", "NoSchedule");
        let b = taint("k1", "DIFFERENT", "NoSchedule");
        let c = taint("k1", "v1", "NoExecute");
        assert!(a.match_taint(&b)); // value ignored
        assert!(!a.match_taint(&c)); // effect differs
    }

    #[test]
    fn taint_to_string_variants() {
        assert_eq!(taint("k", "", "").to_string(), "k");
        assert_eq!(taint("k", "v", "").to_string(), "k=v:");
        assert_eq!(taint("k", "", "NoSchedule").to_string(), "k:NoSchedule");
        assert_eq!(taint("k", "v", "NoSchedule").to_string(), "k=v:NoSchedule");
    }

    #[test]
    fn taint_exists_uses_match_taint() {
        let taints = vec![taint("k1", "v1", "NoSchedule")];
        assert!(taint_exists(&taints, &taint("k1", "other", "NoSchedule")));
        assert!(!taint_exists(&taints, &taint("k1", "v1", "NoExecute")));
    }

    #[test]
    fn add_or_update_appends_new_taint() {
        let taints = vec![taint("k1", "v1", "NoSchedule")];
        let (out, changed) = add_or_update_taint(&taints, &taint("k2", "v2", "NoExecute"));
        assert!(changed);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], taint("k2", "v2", "NoExecute"));
    }

    #[test]
    fn add_or_update_replaces_changed_value() {
        let taints = vec![taint("k1", "v1", "NoSchedule")];
        let (out, changed) = add_or_update_taint(&taints, &taint("k1", "v2", "NoSchedule"));
        assert!(changed);
        assert_eq!(out, vec![taint("k1", "v2", "NoSchedule")]);
    }

    #[test]
    fn add_or_update_identical_is_unchanged() {
        let taints = vec![taint("k1", "v1", "NoSchedule")];
        let (out, changed) = add_or_update_taint(&taints, &taint("k1", "v1", "NoSchedule"));
        assert!(!changed);
        assert_eq!(out, taints);
    }

    #[test]
    fn delete_taint_removes_all_matches() {
        let taints = vec![
            taint("k1", "v1", "NoSchedule"),
            taint("k2", "v2", "NoExecute"),
            taint("k1", "ignored", "NoSchedule"),
        ];
        let (out, deleted) = delete_taint(&taints, &taint("k1", "x", "NoSchedule"));
        assert!(deleted);
        assert_eq!(out, vec![taint("k2", "v2", "NoExecute")]);
    }

    #[test]
    fn delete_taint_absent_reports_false() {
        let taints = vec![taint("k1", "v1", "NoSchedule")];
        let (out, deleted) = delete_taint(&taints, &taint("zzz", "", "NoSchedule"));
        assert!(!deleted);
        assert_eq!(out, taints);
    }

    #[test]
    fn remove_taint_empty_list_is_noop() {
        let (out, changed) = remove_taint(&[], &taint("k1", "", "NoSchedule"));
        assert!(!changed);
        assert!(out.is_empty());
    }

    #[test]
    fn remove_taint_absent_is_noop() {
        let taints = vec![taint("k1", "v1", "NoSchedule")];
        let (out, changed) = remove_taint(&taints, &taint("nope", "", "NoSchedule"));
        assert!(!changed);
        assert_eq!(out, taints);
    }

    #[test]
    fn remove_taint_present_deletes_and_reports_true() {
        let taints = vec![
            taint("k1", "v1", "NoSchedule"),
            taint("k2", "v2", "NoExecute"),
        ];
        let (out, changed) = remove_taint(&taints, &taint("k1", "irrelevant", "NoSchedule"));
        assert!(changed);
        assert_eq!(out, vec![taint("k2", "v2", "NoExecute")]);
    }
}
