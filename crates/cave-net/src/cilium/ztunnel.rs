// SPDX-License-Identifier: AGPL-3.0-or-later
//! ztunnel cross-crate cite.
//!
//! Mirrors `pkg/ztunnel/`. The full ztunnel/HBONE state machine lives in
//! `cave-mesh` (the mesh-data-plane crate). This module exists so the
//! parity manifest can record that `pkg/ztunnel/` is mapped, and so any
//! cave-net consumer that needs the wire constants for HBONE-over-mTLS
//! has a place to look.
//!
//! The constants here are the upstream-agreed wire identifiers; the
//! actual proxy behaviour lives in `cave-mesh::ztunnel`.

use crate::cilium::types::Cite;

/// HBONE — HTTP Based Overlay Network Environment. Mirrors the
/// upstream HBONE protocol identifier the agent advertises through xDS.
pub const HBONE_PROTOCOL_ID: &str = "hbone";

/// HBONE inner CONNECT pseudo-method. Mirrors the upstream constant.
pub const HBONE_CONNECT_METHOD: &str = "CONNECT";

/// Listener port for HBONE inbound (mTLS terminate). Mirrors the
/// well-known ztunnel inbound port.
pub const HBONE_INBOUND_PORT: u16 = 15008;

/// Listener port for HBONE outbound (mTLS originate). Mirrors the
/// well-known ztunnel outbound port.
pub const HBONE_OUTBOUND_PORT: u16 = 15001;

/// Well-known SAN URI suffix for ztunnel SPIFFE IDs. Mirrors the
/// upstream Istio SPIFFE format.
pub const SPIFFE_TRUST_DOMAIN_SUFFIX: &str = "/ns/";

/// Cross-crate pointer note. The actual ztunnel implementation lives at
/// `cave-mesh/src/ztunnel/`.
pub const CAVE_MESH_ZTUNNEL_PATH: &str = "cave-mesh/src/ztunnel/";

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::cilium("pkg/ztunnel/", "Constants");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cilium_test_ctx;

    #[test]
    fn hbone_protocol_id_matches_upstream() {
        let (_c, _t) = cilium_test_ctx!("pkg/ztunnel/", "HBONE.ID", "tenant-zt-id");
        assert_eq!(HBONE_PROTOCOL_ID, "hbone");
    }

    #[test]
    fn hbone_connect_method_is_uppercase() {
        let (_c, _t) = cilium_test_ctx!("pkg/ztunnel/", "HBONE.Connect", "tenant-zt-c");
        assert_eq!(HBONE_CONNECT_METHOD, "CONNECT");
    }

    #[test]
    fn hbone_ports_are_well_known_ztunnel_ports() {
        let (_c, _t) = cilium_test_ctx!("pkg/ztunnel/", "HBONE.Ports", "tenant-zt-p");
        assert_eq!(HBONE_INBOUND_PORT, 15008);
        assert_eq!(HBONE_OUTBOUND_PORT, 15001);
    }

    #[test]
    fn spiffe_trust_domain_suffix_matches_istio_format() {
        let (_c, _t) = cilium_test_ctx!("pkg/ztunnel/", "SPIFFE.Suffix", "tenant-zt-s");
        assert_eq!(SPIFFE_TRUST_DOMAIN_SUFFIX, "/ns/");
    }

    #[test]
    fn cave_mesh_pointer_documents_cross_crate_owner() {
        let (_c, _t) = cilium_test_ctx!("pkg/ztunnel/", "CrossCrate", "tenant-zt-cc");
        // This is a documentation breadcrumb, not a path expected to
        // exist relative to this crate.
        assert!(CAVE_MESH_ZTUNNEL_PATH.contains("cave-mesh"));
    }
}
