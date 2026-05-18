// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reserved identity full table — well-known IDs 1..256.
//!
//! Mirrors `pkg/identity/numericidentity.go`. Cilium reserves identity
//! numbers 1..256 for hard-coded entities (host, world, kube-apiserver,
//! ingress, etc). Any rule that mentions an entity name resolves to
//! the corresponding identity here.

use crate::cilium::types::{Cite, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ReservedIdentity {
    Unknown,
    Host,
    World,
    Unmanaged,
    Health,
    Init,
    RemoteNode,
    KubeApiServer,
    Ingress,
    WorldIPv4,
    WorldIPv6,
    EncryptedOverlay,
}

impl ReservedIdentity {
    /// Numeric ID per upstream `pkg/identity/numericidentity.go`.
    pub fn numeric(self) -> u32 {
        match self {
            ReservedIdentity::Unknown => 0,
            ReservedIdentity::Host => 1,
            ReservedIdentity::World => 2,
            ReservedIdentity::Unmanaged => 3,
            ReservedIdentity::Health => 4,
            ReservedIdentity::Init => 5,
            ReservedIdentity::RemoteNode => 6,
            ReservedIdentity::KubeApiServer => 7,
            ReservedIdentity::Ingress => 8,
            ReservedIdentity::WorldIPv4 => 9,
            ReservedIdentity::WorldIPv6 => 10,
            ReservedIdentity::EncryptedOverlay => 11,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ReservedIdentity::Unknown => "reserved:unknown",
            ReservedIdentity::Host => "reserved:host",
            ReservedIdentity::World => "reserved:world",
            ReservedIdentity::Unmanaged => "reserved:unmanaged",
            ReservedIdentity::Health => "reserved:health",
            ReservedIdentity::Init => "reserved:init",
            ReservedIdentity::RemoteNode => "reserved:remote-node",
            ReservedIdentity::KubeApiServer => "reserved:kube-apiserver",
            ReservedIdentity::Ingress => "reserved:ingress",
            ReservedIdentity::WorldIPv4 => "reserved:world-ipv4",
            ReservedIdentity::WorldIPv6 => "reserved:world-ipv6",
            ReservedIdentity::EncryptedOverlay => "reserved:encrypted-overlay",
        }
    }
    pub fn from_numeric(n: u32) -> Option<Self> {
        Some(match n {
            0 => ReservedIdentity::Unknown,
            1 => ReservedIdentity::Host,
            2 => ReservedIdentity::World,
            3 => ReservedIdentity::Unmanaged,
            4 => ReservedIdentity::Health,
            5 => ReservedIdentity::Init,
            6 => ReservedIdentity::RemoteNode,
            7 => ReservedIdentity::KubeApiServer,
            8 => ReservedIdentity::Ingress,
            9 => ReservedIdentity::WorldIPv4,
            10 => ReservedIdentity::WorldIPv6,
            11 => ReservedIdentity::EncryptedOverlay,
            _ => return None,
        })
    }
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "reserved:unknown" => Some(ReservedIdentity::Unknown),
            "reserved:host" => Some(ReservedIdentity::Host),
            "reserved:world" => Some(ReservedIdentity::World),
            "reserved:unmanaged" => Some(ReservedIdentity::Unmanaged),
            "reserved:health" => Some(ReservedIdentity::Health),
            "reserved:init" => Some(ReservedIdentity::Init),
            "reserved:remote-node" => Some(ReservedIdentity::RemoteNode),
            "reserved:kube-apiserver" => Some(ReservedIdentity::KubeApiServer),
            "reserved:ingress" => Some(ReservedIdentity::Ingress),
            "reserved:world-ipv4" => Some(ReservedIdentity::WorldIPv4),
            "reserved:world-ipv6" => Some(ReservedIdentity::WorldIPv6),
            "reserved:encrypted-overlay" => Some(ReservedIdentity::EncryptedOverlay),
            _ => None,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReservedError {
    #[error("identity {0} is not reserved (must be 0..256)")]
    NotReserved(u32),
    #[error("tenant {tenant} cannot mutate reserved table owned by another tenant")]
    TenantDenied { tenant: TenantId },
}

/// Returns the static lookup map of every reserved identity. Useful
/// for serialising `cilium identity list --reserved`.
pub fn full_table() -> BTreeMap<u32, ReservedIdentity> {
    let mut t = BTreeMap::new();
    for r in [
        ReservedIdentity::Unknown,
        ReservedIdentity::Host,
        ReservedIdentity::World,
        ReservedIdentity::Unmanaged,
        ReservedIdentity::Health,
        ReservedIdentity::Init,
        ReservedIdentity::RemoteNode,
        ReservedIdentity::KubeApiServer,
        ReservedIdentity::Ingress,
        ReservedIdentity::WorldIPv4,
        ReservedIdentity::WorldIPv6,
        ReservedIdentity::EncryptedOverlay,
    ] {
        t.insert(r.numeric(), r);
    }
    t
}

/// Returns true if `n` is in the reserved range (1..256).
pub fn is_reserved_range(n: u32) -> bool {
    n < 256
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/identity/numericidentity.go", "ReservedIdentities");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    // ── numeric() round-trip ────────────────────────────────────────────────

    #[test]
    fn numeric_matches_upstream_constants() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Numeric.Constants", "tenant-rid-n");
        assert_eq!(ReservedIdentity::Host.numeric(), 1);
        assert_eq!(ReservedIdentity::World.numeric(), 2);
        assert_eq!(ReservedIdentity::Unmanaged.numeric(), 3);
        assert_eq!(ReservedIdentity::Health.numeric(), 4);
        assert_eq!(ReservedIdentity::Init.numeric(), 5);
        assert_eq!(ReservedIdentity::RemoteNode.numeric(), 6);
        assert_eq!(ReservedIdentity::KubeApiServer.numeric(), 7);
        assert_eq!(ReservedIdentity::Ingress.numeric(), 8);
        assert_eq!(ReservedIdentity::WorldIPv4.numeric(), 9);
        assert_eq!(ReservedIdentity::WorldIPv6.numeric(), 10);
        assert_eq!(ReservedIdentity::EncryptedOverlay.numeric(), 11);
    }

    #[test]
    fn from_numeric_round_trip_for_known() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FromNumeric.RoundTrip", "tenant-rid-fnr");
        for r in [
            ReservedIdentity::Host, ReservedIdentity::World,
            ReservedIdentity::KubeApiServer, ReservedIdentity::Ingress,
            ReservedIdentity::WorldIPv4, ReservedIdentity::WorldIPv6,
            ReservedIdentity::EncryptedOverlay,
        ] {
            assert_eq!(ReservedIdentity::from_numeric(r.numeric()), Some(r));
        }
    }

    #[test]
    fn from_numeric_unknown_returns_none() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FromNumeric.Unknown", "tenant-rid-fnu");
        assert!(ReservedIdentity::from_numeric(99).is_none());
        assert!(ReservedIdentity::from_numeric(255).is_none());
    }

    // ── label() ────────────────────────────────────────────────────────────

    #[test]
    fn label_format_is_reserved_colon_name() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Label.Format", "tenant-rid-l");
        assert_eq!(ReservedIdentity::Host.label(), "reserved:host");
        assert_eq!(ReservedIdentity::World.label(), "reserved:world");
        assert_eq!(ReservedIdentity::KubeApiServer.label(), "reserved:kube-apiserver");
        assert_eq!(ReservedIdentity::WorldIPv4.label(), "reserved:world-ipv4");
        assert_eq!(ReservedIdentity::EncryptedOverlay.label(), "reserved:encrypted-overlay");
    }

    #[test]
    fn from_label_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FromLabel.RoundTrip", "tenant-rid-flr");
        for r in [
            ReservedIdentity::Host, ReservedIdentity::World,
            ReservedIdentity::KubeApiServer, ReservedIdentity::Ingress,
            ReservedIdentity::WorldIPv4, ReservedIdentity::WorldIPv6,
        ] {
            assert_eq!(ReservedIdentity::from_label(r.label()), Some(r));
        }
    }

    #[test]
    fn from_label_unknown_returns_none() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FromLabel.Unknown", "tenant-rid-flu");
        assert!(ReservedIdentity::from_label("reserved:nope").is_none());
        assert!(ReservedIdentity::from_label("nonsense").is_none());
    }

    // ── full_table() ───────────────────────────────────────────────────────

    #[test]
    fn full_table_contains_all_known_identities() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FullTable", "tenant-rid-ft");
        let t = full_table();
        assert!(t.contains_key(&1));
        assert!(t.contains_key(&7));
        assert!(t.contains_key(&8));
        assert!(t.contains_key(&11));
    }

    #[test]
    fn full_table_count_is_twelve() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FullTable.Count", "tenant-rid-ftc");
        let t = full_table();
        assert_eq!(t.len(), 12); // unknown + 11 named
    }

    #[test]
    fn full_table_keys_distinct() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "FullTable.Distinct", "tenant-rid-ftd");
        let t = full_table();
        let n = t.len();
        assert_eq!(t.values().collect::<std::collections::BTreeSet<_>>().len(), n);
    }

    // ── is_reserved_range ──────────────────────────────────────────────────

    #[test]
    fn is_reserved_range_below_256() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Range.Reserved", "tenant-rid-rr");
        assert!(is_reserved_range(0));
        assert!(is_reserved_range(1));
        assert!(is_reserved_range(255));
    }

    #[test]
    fn is_reserved_range_above_256_is_false() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Range.Local", "tenant-rid-rl");
        assert!(!is_reserved_range(256));
        assert!(!is_reserved_range(1024));
    }

    // ── Ordering ───────────────────────────────────────────────────────────

    #[test]
    fn reserved_identity_ordered_by_numeric_value() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Ordering", "tenant-rid-o");
        // Verify the enum ordering matches numeric ordering.
        let mut all = vec![
            ReservedIdentity::EncryptedOverlay,
            ReservedIdentity::Host,
            ReservedIdentity::World,
            ReservedIdentity::Unknown,
        ];
        all.sort_by_key(|r| r.numeric());
        assert_eq!(all[0], ReservedIdentity::Unknown);
        assert_eq!(all[1], ReservedIdentity::Host);
    }

    // ── Serde ──────────────────────────────────────────────────────────────

    #[test]
    fn reserved_identity_serde_round_trip() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Serde", "tenant-rid-serde");
        for r in [
            ReservedIdentity::Host, ReservedIdentity::World,
            ReservedIdentity::KubeApiServer, ReservedIdentity::Ingress,
            ReservedIdentity::EncryptedOverlay,
        ] {
            let s = serde_json::to_string(&r).unwrap();
            let back: ReservedIdentity = serde_json::from_str(&s).unwrap();
            assert_eq!(back, r);
        }
    }

    // ── Spot-checks on specific identities ─────────────────────────────────

    #[test]
    fn ingress_identity_is_eight() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "Ingress", "tenant-rid-ing");
        assert_eq!(ReservedIdentity::Ingress.numeric(), 8);
        assert_eq!(ReservedIdentity::Ingress.label(), "reserved:ingress");
    }

    #[test]
    fn kube_apiserver_identity_is_seven() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "KubeAPIServer", "tenant-rid-kas");
        assert_eq!(ReservedIdentity::KubeApiServer.numeric(), 7);
        assert_eq!(ReservedIdentity::KubeApiServer.label(), "reserved:kube-apiserver");
    }

    #[test]
    fn world_split_into_v4_and_v6() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "WorldSplit", "tenant-rid-ws");
        assert_eq!(ReservedIdentity::WorldIPv4.numeric(), 9);
        assert_eq!(ReservedIdentity::WorldIPv6.numeric(), 10);
        assert_eq!(ReservedIdentity::WorldIPv4.label(), "reserved:world-ipv4");
        assert_eq!(ReservedIdentity::WorldIPv6.label(), "reserved:world-ipv6");
    }

    #[test]
    fn encrypted_overlay_identity_is_eleven() {
        let (_c, _t) = cilium_test_ctx!("pkg/identity/numericidentity.go", "EncryptedOverlay", "tenant-rid-eo");
        assert_eq!(ReservedIdentity::EncryptedOverlay.numeric(), 11);
    }
}
