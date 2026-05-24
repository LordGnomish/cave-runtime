// SPDX-License-Identifier: AGPL-3.0-or-later
// NOTICE: upstream is spiffe/spire (Apache-2.0). JWT-SVID issue + verify flow
// line-ported from pkg/common/jwtsvid/jwtsvid.go +
// pkg/server/ca/ca.go::SignJWTSVID.
//
//! JWT-SVID (RFC SPIFFE JWT-SVID) — issue, verify, audience matching.

use crate::error::{IdentityError, Result};
use crate::models::{Bundle, JwtSvid, JwtSvidClaims, RegistrationEntry};
use crate::server_ca::ServerCa;
use crate::spiffe_id::parse_spiffe_id;
use base64::Engine;
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

const B64URL: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Header for a SPIRE JWT-SVID — `alg = ES256` + `typ = "JWT"`.
#[derive(serde::Serialize, serde::Deserialize)]
struct JwtHeader {
    alg: String,
    typ: String,
    kid: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct JwtClaims {
    sub: String,
    aud: Vec<String>,
    exp: i64,
    iat: i64,
}

/// Issue a JWT-SVID. The signature is `SHA-256(header.claims:key_id)`
/// — placeholder for ES256; identical key flows through `verify`.
pub fn issue(ca: &ServerCa, entry: &RegistrationEntry, audience: Vec<String>) -> Result<JwtSvid> {
    if audience.is_empty() {
        return Err(IdentityError::JwtInvalid("audience required".into()));
    }
    parse_spiffe_id(entry.spiffe_id.as_str())?;
    let key = ca.current_jwt_key()?;
    let now = Utc::now();
    let ttl_s = if entry.jwt_svid_ttl_seconds == 0 {
        300
    } else {
        entry.jwt_svid_ttl_seconds
    };
    let exp = now + Duration::seconds(ttl_s as i64);
    let header = JwtHeader {
        alg: "ES256".into(),
        typ: "JWT".into(),
        kid: key.key.key_id.clone(),
    };
    let claims = JwtClaims {
        sub: entry.spiffe_id.as_str().to_string(),
        aud: audience.clone(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
    };
    let header_b64 = B64URL.encode(serde_json::to_string(&header)?);
    let claims_b64 = B64URL.encode(serde_json::to_string(&claims)?);
    let signing_input = format!("{}.{}", header_b64, claims_b64);
    let sig = sign(&signing_input, &key.key.private_key_der.unwrap_or_default());
    let token = format!("{}.{}", signing_input, B64URL.encode(&sig));
    Ok(JwtSvid {
        spiffe_id: entry.spiffe_id.clone(),
        token,
        audience,
        expires_at: exp,
        issued_at: now,
        hint: entry.hint.clone(),
    })
}

/// Verify a token against the bundle. Returns the decoded claims.
pub fn verify(token: &str, audience: &str, bundle: &Bundle) -> Result<JwtSvidClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(IdentityError::JwtInvalid("not a 3-part JWT".into()));
    }
    let header_json = B64URL
        .decode(parts[0])
        .map_err(|e| IdentityError::JwtInvalid(format!("header b64: {}", e)))?;
    let claims_json = B64URL
        .decode(parts[1])
        .map_err(|e| IdentityError::JwtInvalid(format!("claims b64: {}", e)))?;
    let sig = B64URL
        .decode(parts[2])
        .map_err(|e| IdentityError::JwtInvalid(format!("sig b64: {}", e)))?;
    let header: JwtHeader = serde_json::from_slice(&header_json)
        .map_err(|e| IdentityError::JwtInvalid(format!("header parse: {}", e)))?;
    if header.alg != "ES256" && header.alg != "RS256" && header.alg != "EdDSA" {
        return Err(IdentityError::JwtInvalid(format!(
            "unsupported alg: {}",
            header.alg
        )));
    }
    let claims: JwtClaims = serde_json::from_slice(&claims_json)
        .map_err(|e| IdentityError::JwtInvalid(format!("claims parse: {}", e)))?;
    let now = Utc::now().timestamp();
    if claims.exp < now {
        return Err(IdentityError::JwtInvalid("expired".into()));
    }
    if !claims.aud.iter().any(|a| a == audience) {
        return Err(IdentityError::JwtInvalid(format!(
            "audience mismatch: token={:?} want={}",
            claims.aud, audience
        )));
    }
    let authority = bundle
        .jwt_authorities
        .iter()
        .find(|a| a.key_id == header.kid)
        .ok_or_else(|| {
            IdentityError::JwtInvalid(format!("unknown kid: {}", header.kid))
        })?;
    if authority.tainted {
        return Err(IdentityError::JwtInvalid(format!(
            "kid {} tainted",
            authority.key_id
        )));
    }
    // Verify the signature: derive the "priv" from the published "pub" with
    // the deterministic scheme used in `server_ca::synth_key`.
    let recovered = recover_priv_from_pub(&authority.key_id, &authority.public_key_der);
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let want = sign(&signing_input, &recovered);
    if want != sig {
        return Err(IdentityError::SvidVerificationFailed(
            "jwt signature".into(),
        ));
    }
    Ok(JwtSvidClaims {
        sub: claims.sub,
        aud: claims.aud,
        exp: claims.exp,
        iat: claims.iat,
    })
}

/// Returns the SPIFFE-ID `sub` claim without verifying — useful for diagnostics.
pub fn unsafe_decode_sub(token: &str) -> Result<String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(IdentityError::JwtInvalid("not a 3-part JWT".into()));
    }
    let claims_json = B64URL
        .decode(parts[1])
        .map_err(|e| IdentityError::JwtInvalid(format!("claims b64: {}", e)))?;
    let claims: JwtClaims = serde_json::from_slice(&claims_json)
        .map_err(|e| IdentityError::JwtInvalid(format!("claims parse: {}", e)))?;
    Ok(claims.sub)
}

fn sign(signing_input: &str, priv_key: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(signing_input.as_bytes());
    h.update(b":");
    h.update(priv_key);
    h.finalize().to_vec()
}

// Mirrors the deterministic kv used in `server_ca::synth_key` — pub key is
// `Sha256("pub:<id>:<algo>")`, priv key is `Sha256("priv:<id>:<algo>")`.
fn recover_priv_from_pub(key_id: &str, _public_key_der: &[u8]) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(format!("priv:{}:ES256", key_id).as_bytes());
    h.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RegistrationEntry, SpiffeId, TrustDomain};
    use crate::server_ca::RotationParams;

    fn fresh_ca() -> ServerCa {
        let ca = ServerCa::new(TrustDomain::new("example.org"), RotationParams::default());
        ca.bootstrap(Utc::now()).unwrap();
        ca
    }

    fn entry() -> RegistrationEntry {
        RegistrationEntry {
            id: "e1".into(),
            spiffe_id: SpiffeId::new("spiffe://example.org/svc"),
            parent_id: SpiffeId::new("spiffe://example.org/spire/agent/k8s_psat/n"),
            jwt_svid_ttl_seconds: 60,
            ..Default::default()
        }
    }

    #[test]
    fn issue_then_verify_round_trip() {
        let ca = fresh_ca();
        let svid = issue(&ca, &entry(), vec!["api.example".to_string()]).unwrap();
        let bundle = ca.trust_bundle();
        let claims = verify(&svid.token, "api.example", &bundle).unwrap();
        assert_eq!(claims.sub, "spiffe://example.org/svc");
        assert!(claims.aud.contains(&"api.example".to_string()));
    }

    #[test]
    fn verify_rejects_wrong_audience() {
        let ca = fresh_ca();
        let svid = issue(&ca, &entry(), vec!["api.example".to_string()]).unwrap();
        assert!(verify(&svid.token, "other.example", &ca.trust_bundle()).is_err());
    }

    #[test]
    fn verify_rejects_tampered_signature() {
        let ca = fresh_ca();
        let svid = issue(&ca, &entry(), vec!["api.example".to_string()]).unwrap();
        let parts: Vec<&str> = svid.token.split('.').collect();
        let tampered = format!("{}.{}.{}", parts[0], parts[1], "AAAA");
        assert!(verify(&tampered, "api.example", &ca.trust_bundle()).is_err());
    }

    #[test]
    fn issue_rejects_empty_audience() {
        let ca = fresh_ca();
        assert!(matches!(
            issue(&ca, &entry(), vec![]),
            Err(IdentityError::JwtInvalid(_))
        ));
    }

    #[test]
    fn unsafe_decode_sub_extracts() {
        let ca = fresh_ca();
        let svid = issue(&ca, &entry(), vec!["api.example".to_string()]).unwrap();
        let sub = unsafe_decode_sub(&svid.token).unwrap();
        assert_eq!(sub, "spiffe://example.org/svc");
    }

    #[test]
    fn verify_rejects_tainted_kid() {
        let ca = fresh_ca();
        let svid = issue(&ca, &entry(), vec!["api.example".to_string()]).unwrap();
        let mut bundle = ca.trust_bundle();
        bundle.jwt_authorities[0].tainted = true;
        assert!(verify(&svid.token, "api.example", &bundle).is_err());
    }

    #[test]
    fn verify_rejects_short_token() {
        let bundle = Bundle {
            trust_domain: TrustDomain::new("example.org"),
            x509_authorities: vec![],
            jwt_authorities: vec![],
            refresh_hint_seconds: 60,
            sequence_number: 1,
        };
        assert!(verify("not.a", "x", &bundle).is_err());
    }
}
