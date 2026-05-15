// SPDX-License-Identifier: AGPL-3.0-or-later
//
// JWK Thumbprint — RFC 7638.
//
// The DPoP confirmation claim `cnf.jkt` (RFC 9449 §6.1) is the base64url(no
// pad) SHA-256 of the JSON Web Key with members in lexicographic order and
// with only the required members for that key type.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::DpopError;

/// Compute the RFC 7638 SHA-256 thumbprint of a JWK encoded as JSON.
///
/// The input `jwk` is the un-ordered JSON; this function picks the canonical
/// members per key type, serialises them in lexicographic order, and hashes.
pub fn jkt(jwk: &serde_json::Value) -> Result<String, DpopError> {
    let kty = jwk.get("kty").and_then(|v| v.as_str()).ok_or(DpopError::Proof("kty missing"))?;
    let canonical = match kty {
        "RSA" => {
            #[derive(Deserialize)]
            struct Rsa { e: String, kty: String, n: String }
            let r: Rsa = serde_json::from_value(jwk.clone()).map_err(|e| DpopError::Json(e.to_string()))?;
            format!(r#"{{"e":"{}","kty":"{}","n":"{}"}}"#, r.e, r.kty, r.n)
        }
        "EC" => {
            #[derive(Deserialize)]
            struct Ec { crv: String, kty: String, x: String, y: String }
            let e: Ec = serde_json::from_value(jwk.clone()).map_err(|e| DpopError::Json(e.to_string()))?;
            format!(
                r#"{{"crv":"{}","kty":"{}","x":"{}","y":"{}"}}"#,
                e.crv, e.kty, e.x, e.y
            )
        }
        "OKP" => {
            #[derive(Deserialize)]
            struct Okp { crv: String, kty: String, x: String }
            let o: Okp = serde_json::from_value(jwk.clone()).map_err(|e| DpopError::Json(e.to_string()))?;
            format!(r#"{{"crv":"{}","kty":"{}","x":"{}"}}"#, o.crv, o.kty, o.x)
        }
        other => return Err(DpopError::UnsupportedAlg(format!("kty={other}"))),
    };
    let digest = Sha256::digest(canonical.as_bytes());
    Ok(B64.encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    // upstream: rfc7638 §3.1 — the example RSA JWK in the appendix has
    // thumbprint "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs".
    #[test]
    fn rfc7638_rsa_example_thumbprint() {
        let jwk = serde_json::json!({
            "kty": "RSA",
            "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
            "e": "AQAB",
            "alg": "RS256",
            "kid": "2011-04-29"
        });
        let t = jkt(&jwk).unwrap();
        assert_eq!(t, "NzbLsXh8uDCcd-6MNwXF4W_7noWXFZAfHkxZsRGC9Xs");
    }

    // upstream: rfc7638 §3 — only the canonical members are hashed; "kid" and
    // "alg" must not change the thumbprint.
    #[test]
    fn kid_and_alg_do_not_affect_thumbprint() {
        let base = serde_json::json!({
            "kty": "RSA",
            "n": "0vx7agoebGcQSuuPiLJXZptN9nndrQmbXEps2aiAFbWhM78LhWx4cbbfAAtVT86zwu1RK7aPFFxuhDR1L6tSoc_BJECPebWKRXjBZCiFV4n3oknjhMstn64tZ_2W-5JsGY4Hc5n9yBXArwl93lqt7_RN5w6Cf0h4QyQ5v-65YGjQR0_FDW2QvzqY368QQMicAtaSqzs8KJZgnYb9c7d0zgdAZHzu6qMQvRL5hajrn1n91CbOpbISD08qNLyrdkt-bFTWhAI4vMQFh6WeZu0fM4lFd2NcRwr3XPksINHaQ-G_xBniIqbw0Ls1jF44-csFCur-kEgU8awapJzKnqDKgw",
            "e": "AQAB"
        });
        let mut with_meta = base.clone();
        with_meta["kid"] = "anything".into();
        with_meta["alg"] = "RS256".into();
        assert_eq!(jkt(&base).unwrap(), jkt(&with_meta).unwrap());
    }

    // upstream: rfc7638 §3.1 — EC P-256 keys hash on (crv, kty, x, y).
    #[test]
    fn ec_p256_thumbprint_is_stable() {
        let jwk = serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "x": "MKBCTNIcKUSDii11ySs3526iDZ8AiTo7Tu6KPAqv7D4",
            "y": "4Etl6SRW2YiLUrN5vfvVHuhp7x8PxltmWWlbbM4IFyM"
        });
        let t1 = jkt(&jwk).unwrap();
        let t2 = jkt(&jwk).unwrap();
        assert_eq!(t1, t2);
        // sha256 base64url-no-pad is 43 chars (32 bytes).
        assert_eq!(t1.len(), 43);
    }

    // upstream: rfc9449 §6.1 — `cnf.jkt` rejects unsupported kty.
    #[test]
    fn unknown_kty_errors() {
        let jwk = serde_json::json!({"kty":"oct","k":"..."});
        assert!(jkt(&jwk).is_err());
    }
}
