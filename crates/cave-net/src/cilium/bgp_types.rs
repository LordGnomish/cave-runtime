// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! BGPv1 / BGPv2 shared types — root pkg/bgp/.
//!
//! Mirrors `pkg/bgp/cell.go` and the small CRD-shaped types used across
//! the BGP feature area. The per-instance BGP daemon lives in
//! [`crate::cilium::bgp`] (which mirrors `pkg/bgpv1/manager/manager.go`).
//! This module captures only the cross-cutting types that aren't tied
//! to a specific implementation.

use crate::cilium::types::Cite;
use serde::{Deserialize, Serialize};

/// AFI/SAFI pair Cilium advertises. The agent-side BGP daemon supports
/// IPv4-Unicast, IPv6-Unicast, and L2VPN-EVPN (the latter via labelled
/// SRv6 advertisements).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AfiSafi {
    Ipv4Unicast,
    Ipv6Unicast,
    L2vpnEvpn,
}

impl AfiSafi {
    pub fn afi(self) -> u16 {
        match self { AfiSafi::Ipv4Unicast => 1, AfiSafi::Ipv6Unicast => 2, AfiSafi::L2vpnEvpn => 25 }
    }
    pub fn safi(self) -> u8 {
        match self { AfiSafi::Ipv4Unicast | AfiSafi::Ipv6Unicast => 1, AfiSafi::L2vpnEvpn => 70 }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            AfiSafi::Ipv4Unicast => "ipv4-unicast",
            AfiSafi::Ipv6Unicast => "ipv6-unicast",
            AfiSafi::L2vpnEvpn => "l2vpn-evpn",
        }
    }
}

/// A peer described in `CiliumBGPAdvertisement` / `CiliumBGPNodeConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerSpec {
    pub name: String,
    pub asn: u32,
    pub address: String,
    /// Optional MD5 password for the BGP session.
    pub password: Option<String>,
}

/// One advertisement entry: a route to announce with optional attributes.
/// Mirrors the `CiliumBGPAdvertisement` CRD shape from `pkg/bgp/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advertisement {
    pub prefix: String,        // CIDR
    pub afi_safi: AfiSafi,
    pub local_pref: Option<u32>,
    pub communities: Vec<String>, // standard / large communities
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/bgp/cell.go", "Cell");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn afi_safi_iana_codes() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "AfiSafi.Codes", "tenant-bgpr-codes");
        // RFC 4760 + 7432
        assert_eq!(AfiSafi::Ipv4Unicast.afi(), 1);
        assert_eq!(AfiSafi::Ipv4Unicast.safi(), 1);
        assert_eq!(AfiSafi::Ipv6Unicast.afi(), 2);
        assert_eq!(AfiSafi::Ipv6Unicast.safi(), 1);
        assert_eq!(AfiSafi::L2vpnEvpn.afi(), 25);
        assert_eq!(AfiSafi::L2vpnEvpn.safi(), 70);
    }

    #[test]
    fn afi_safi_as_str() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "AfiSafi.AsStr", "tenant-bgpr-str");
        assert_eq!(AfiSafi::Ipv4Unicast.as_str(), "ipv4-unicast");
        assert_eq!(AfiSafi::Ipv6Unicast.as_str(), "ipv6-unicast");
        assert_eq!(AfiSafi::L2vpnEvpn.as_str(), "l2vpn-evpn");
    }

    #[test]
    fn peer_spec_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "PeerSpec.Serde", "tenant-bgpr-ps");
        let p = PeerSpec {
            name: "tor-1".into(),
            asn: 65001,
            address: "10.0.0.1".into(),
            password: Some("hunter2".into()),
        };
        let s = serde_json::to_string(&p).unwrap();
        let back: PeerSpec = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn advertisement_with_communities() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "Adv.Communities", "tenant-bgpr-com");
        let a = Advertisement {
            prefix: "10.0.0.0/24".into(),
            afi_safi: AfiSafi::Ipv4Unicast,
            local_pref: Some(100),
            communities: vec!["65000:1".into(), "no-export".into()],
        };
        assert_eq!(a.communities.len(), 2);
        assert_eq!(a.local_pref, Some(100));
    }

    #[test]
    fn afi_safi_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "AfiSafi.Serde", "tenant-bgpr-srd");
        for v in [AfiSafi::Ipv4Unicast, AfiSafi::Ipv6Unicast, AfiSafi::L2vpnEvpn] {
            let s = serde_json::to_string(&v).unwrap();
            let back: AfiSafi = serde_json::from_str(&s).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn peer_without_password_is_serializable() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "PeerSpec.NoPwd", "tenant-bgpr-pn");
        let p = PeerSpec { name: "p".into(), asn: 65000, address: "1.1.1.1".into(), password: None };
        let s = serde_json::to_string(&p).unwrap();
        assert!(s.contains("\"password\":null"));
    }

    #[test]
    fn advertisement_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "Adv.Serde", "tenant-bgpr-as");
        let a = Advertisement {
            prefix: "fd00::/8".into(),
            afi_safi: AfiSafi::Ipv6Unicast,
            local_pref: None,
            communities: vec![],
        };
        let s = serde_json::to_string(&a).unwrap();
        let back: Advertisement = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }

    #[test]
    fn evpn_safi_is_seventy() {
        let (_c, _t) = cilium_test_ctx!("pkg/bgp/cell.go", "AfiSafi.EVPN", "tenant-bgpr-evpn");
        // EVPN SAFI is 70 per RFC 7432.
        assert_eq!(AfiSafi::L2vpnEvpn.safi(), 70);
    }
}
