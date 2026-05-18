// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + keycloak/keycloak v22.0.0 services/.../protocol/oid4vc/ +
// W3C VC 2.0 + OID4VCI + OID4VP.

//! `cavectl auth oid4vc` — OpenID for Verifiable Credentials issuance +
//! verification. Parity surface tracked by `crates/cave-auth/src/oid4vc/`.

/// `cavectl auth oid4vc issue` — OID4VCI offer + nonce status.
pub const PATH_ISSUE: &str = "/api/auth/oid4vc/issue";

/// `cavectl auth oid4vc credential` — `/credential` endpoint surface.
pub const PATH_CREDENTIAL: &str = "/api/auth/oid4vc/credential";

/// `cavectl auth oid4vc present` — OID4VP presentation request status.
pub const PATH_PRESENT: &str = "/api/auth/oid4vc/present";

/// `cavectl auth oid4vc metadata` — issuer + verifier metadata.
pub const PATH_METADATA: &str = "/api/auth/oid4vc/metadata";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_oid4vc_prefix() {
        for p in [PATH_ISSUE, PATH_CREDENTIAL, PATH_PRESENT, PATH_METADATA] {
            assert!(p.starts_with("/api/auth/oid4vc/"));
        }
    }
}
