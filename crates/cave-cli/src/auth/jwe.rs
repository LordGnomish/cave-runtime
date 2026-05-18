// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: cave-cli + RFC 7516 (JWE) + RFC 7518 (JWA).

//! `cavectl auth jwe` — JWE encrypt / decrypt admin surface.
//! Parity tracked by `crates/cave-auth/src/jwe/`.

/// `cavectl auth jwe encrypt` — RSA-OAEP + A256GCM compact JWE issuance.
pub const PATH_ENCRYPT: &str = "/api/auth/jwe/encrypt";

/// `cavectl auth jwe decrypt` — decrypt + protected-header inspect.
pub const PATH_DECRYPT: &str = "/api/auth/jwe/decrypt";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_share_jwe_prefix() {
        for p in [PATH_ENCRYPT, PATH_DECRYPT] {
            assert!(p.starts_with("/api/auth/jwe/"));
        }
    }
}
