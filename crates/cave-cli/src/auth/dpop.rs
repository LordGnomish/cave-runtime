// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + RFC 9449 (DPoP) + RFC 7638 (JWK Thumbprint).

//! `cavectl auth dpop` — DPoP proof verify + thumbprint compute admin.
//! Parity surface tracked by `crates/cave-auth/src/dpop/`.

/// `cavectl auth dpop verify-proof` — parse + verify a DPoP JWT against an
/// access-token's `cnf.jkt` thumbprint.
pub const PATH_VERIFY_PROOF: &str = "/api/auth/dpop/verify-proof";

/// `cavectl auth dpop thumbprint` — compute RFC 7638 JWK thumbprint for a key.
pub const PATH_THUMBPRINT: &str = "/api/auth/dpop/thumbprint";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_dpop_prefix() {
        for p in [PATH_VERIFY_PROOF, PATH_THUMBPRINT] {
            assert!(p.starts_with("/api/auth/dpop/"));
        }
    }
}
