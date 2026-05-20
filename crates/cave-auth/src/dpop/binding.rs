// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/+ RFC 7638 (JWK Thumbprint) + RFC 9449
//
//! JWK Thumbprint binding — RFC 7638 §3, used in DPoP's `cnf.jkt` (RFC 9449 §6.1).
//!
//! The thumbprint is `BASE64URL(SHA-256(canonical-JWK))` where the canonical JWK is
//! built from only the REQUIRED members of the key, sorted lexicographically, with
//! no insignificant whitespace.

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Minimal JWK as it appears in a DPoP proof header (`jwk` parameter).
///
/// Only ES256 (EC P-256) and RS256 (RSA) are accepted; the same struct doubles
/// as the canonical thumbprint input.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kty")]
pub enum Jwk {
    /// EC key — RFC 7638 §3.2: required members are `crv`, `kty`, `x`, `y`.
    #[serde(rename = "EC")]
    Ec { crv: String, x: String, y: String },
    /// RSA key — RFC 7638 §3.2: required members are `e`, `kty`, `n`.
    #[serde(rename = "RSA")]
    Rsa { e: String, n: String },
}

impl Jwk {
    /// Returns the canonical JSON form per RFC 7638 §3.1
    /// (members lexicographic, no whitespace, no extras).
    pub fn canonical_json(&self) -> String {
        match self {
            Jwk::Ec { crv, x, y } => {
                format!(r#"{{"crv":"{crv}","kty":"EC","x":"{x}","y":"{y}"}}"#)
            }
            Jwk::Rsa { e, n } => {
                format!(r#"{{"e":"{e}","kty":"RSA","n":"{n}"}}"#)
            }
        }
    }
}

/// Compute the RFC 7638 JWK Thumbprint, base64url-encoded (no padding).
///
/// The output is the value bound into a DPoP-bound access token's `cnf.jkt`
/// claim (RFC 9449 §6.1).
pub fn jkt_thumbprint(jwk: &Jwk) -> String {
    let canonical = jwk.canonical_json();
    let digest = Sha256::digest(canonical.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7638 §3.1 — the example RSA key MUST produce thumbprint
    /// `NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs`.
    #[test]
    fn rfc7638_rsa_thumbprint_vector() {
        let jwk = Jwk::Rsa {
            n: "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw".to_string(),
            e: "AQAB".to_string(),
        };
        let thumbprint = jkt_thumbprint(&jwk);
        assert_eq!(
            thumbprint, "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs",
            "RFC 7638 §3.1 reference vector mismatch"
        );
    }

    #[test]
    fn canonical_json_rsa_lexicographic_order() {
        let jwk = Jwk::Rsa {
            n: "abc".to_string(),
            e: "AQAB".to_string(),
        };
        let canon = jwk.canonical_json();
        // RFC 7638 §3.2: members MUST be in lexicographic order.
        let e_pos = canon.find("\"e\":").unwrap();
        let kty_pos = canon.find("\"kty\":").unwrap();
        let n_pos = canon.find("\"n\":").unwrap();
        assert!(
            e_pos < kty_pos && kty_pos < n_pos,
            "members out of order: {canon}"
        );
    }

    #[test]
    fn canonical_json_ec_lexicographic_order() {
        let jwk = Jwk::Ec {
            crv: "P-256".to_string(),
            x: "AAAA".to_string(),
            y: "BBBB".to_string(),
        };
        let canon = jwk.canonical_json();
        let crv_pos = canon.find("\"crv\":").unwrap();
        let kty_pos = canon.find("\"kty\":").unwrap();
        let x_pos = canon.find("\"x\":").unwrap();
        let y_pos = canon.find("\"y\":").unwrap();
        assert!(crv_pos < kty_pos && kty_pos < x_pos && x_pos < y_pos);
    }

    #[test]
    fn thumbprint_distinct_per_key() {
        let k1 = Jwk::Ec {
            crv: "P-256".to_string(),
            x: "AAAA".to_string(),
            y: "BBBB".to_string(),
        };
        let k2 = Jwk::Ec {
            crv: "P-256".to_string(),
            x: "CCCC".to_string(),
            y: "DDDD".to_string(),
        };
        assert_ne!(jkt_thumbprint(&k1), jkt_thumbprint(&k2));
    }

    #[test]
    fn thumbprint_is_stable() {
        let k = Jwk::Rsa {
            n: "X".to_string(),
            e: "AQAB".to_string(),
        };
        let a = jkt_thumbprint(&k);
        let b = jkt_thumbprint(&k);
        assert_eq!(a, b, "thumbprint must be deterministic");
    }

    #[test]
    fn thumbprint_is_base64url_no_pad() {
        let k = Jwk::Rsa {
            n: "X".to_string(),
            e: "AQAB".to_string(),
        };
        let t = jkt_thumbprint(&k);
        assert!(!t.contains('='), "padding leaked into thumbprint: {t}");
        assert!(!t.contains('+'), "non-url-safe char: {t}");
        assert!(!t.contains('/'), "non-url-safe char: {t}");
    }
}
