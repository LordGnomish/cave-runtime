// SPDX-License-Identifier: AGPL-3.0-or-later
//
// DPoP proof JWT verification — RFC 9449 §4.3.
//
// The verifier:
//   1. Parses the compact JWS in the `DPoP` header (`header.rs`).
//   2. Confirms `typ == "dpop+jwt"` and the JWK is well-formed.
//   3. Verifies the signature using the embedded JWK
//      (RS256 / ES256 / EdDSA).
//   4. Checks `htm` matches the HTTP method.
//   5. Checks `htu` matches the request URI (path-and-query, scheme/host).
//   6. Checks `iat` is within the configured skew window.
//   7. Checks `jti` is unique (replay defense).
//   8. Optionally checks `nonce` (server-issued nonce challenge).
//   9. Optionally checks `ath` (access-token hash) when the proof is sent
//      with a Bearer-style access token.
//
// Upstream: keycloak/keycloak  b825ba97b489d715f7ca1984c19bd95afb355a38
//   services/src/main/java/org/keycloak/protocol/oidc/grants/DPoPHandler.java
//   services/src/main/java/org/keycloak/crypto/DPoPProofValidator.java

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use sha2::{Digest, Sha256};

use super::header::{DpopJwsHeader, DpopProofClaims, ParsedHeader};
use super::nonce::NonceStore;
use super::thumbprint::jkt;
use super::DpopError;

/// Configuration for verifying a DPoP proof.
#[derive(Debug, Clone)]
pub struct VerifyOptions {
    pub expected_htm: String,
    pub expected_htu: String,
    /// `iat` must be within ±`iat_skew_secs` of `now()`.
    pub iat_skew_secs: i64,
    /// If `Some`, the proof MUST carry an `ath` claim equal to
    /// `BASE64URL(SHA256(access_token))`.
    pub expected_ath: Option<String>,
    /// If `Some`, the proof MUST carry a `nonce` claim equal to this value.
    pub expected_nonce: Option<String>,
    /// Current epoch (seconds). Caller passes time so we stay testable.
    pub now_unix: i64,
}

/// Outcome of a successful verification — exposes the JWK thumbprint so the
/// token endpoint can bind the issued access token.
#[derive(Debug, Clone)]
pub struct VerifiedProof {
    pub jkt: String,
    pub claims: DpopProofClaims,
}

/// Verify a DPoP proof against the supplied policy.
pub fn verify(
    raw_dpop: &str,
    opts: &VerifyOptions,
    store: &NonceStore,
) -> Result<VerifiedProof, DpopError> {
    let parsed = ParsedHeader::parse(raw_dpop)?;
    let hdr: DpopJwsHeader = parsed.decode_header()?;
    if hdr.typ != "dpop+jwt" {
        return Err(DpopError::Proof("typ must be dpop+jwt"));
    }
    let claims = parsed.decode_payload()?;

    verify_signature(&parsed, &hdr)?;

    if claims.htm != opts.expected_htm {
        return Err(DpopError::HtmMismatch {
            expected: opts.expected_htm.clone(),
            got: claims.htm.clone(),
        });
    }
    if claims.htu != opts.expected_htu {
        return Err(DpopError::HtuMismatch {
            expected: opts.expected_htu.clone(),
            got: claims.htu.clone(),
        });
    }
    if (claims.iat - opts.now_unix).abs() > opts.iat_skew_secs {
        return Err(DpopError::IatOutOfWindow);
    }
    if claims.jti.is_empty() {
        return Err(DpopError::MissingJti);
    }
    if !store.record_jti(&claims.jti) {
        return Err(DpopError::Replay);
    }
    if let Some(exp) = &opts.expected_nonce {
        if claims.nonce.as_deref() != Some(exp.as_str()) {
            return Err(DpopError::NonceMismatch);
        }
    }
    if let Some(exp_ath) = &opts.expected_ath {
        if claims.ath.as_deref() != Some(exp_ath.as_str()) {
            return Err(DpopError::AthMismatch);
        }
    }

    let jkt = jkt(&hdr.jwk)?;
    Ok(VerifiedProof { jkt, claims })
}

/// Compute `ath` from a raw access token (RFC 9449 §4.2 + §6).
pub fn ath_from_access_token(access_token: &str) -> String {
    let d = Sha256::digest(access_token.as_bytes());
    B64.encode(d)
}

/// Verify the signature segment using the algorithm declared in the header
/// and the JWK published alongside it. Supports RS256, ES256, EdDSA.
fn verify_signature(parsed: &ParsedHeader, hdr: &DpopJwsHeader) -> Result<(), DpopError> {
    let signing_input = parsed.signing_input();
    let sig = parsed.signature_bytes()?;

    match hdr.alg.as_str() {
        "ES256" => verify_es256(&signing_input, &sig, &hdr.jwk),
        "RS256" => verify_rs256(&signing_input, &sig, &hdr.jwk),
        "EdDSA" => verify_eddsa(&signing_input, &sig, &hdr.jwk),
        other => Err(DpopError::UnsupportedAlg(other.to_string())),
    }
}

fn verify_es256(msg: &str, sig: &[u8], jwk: &serde_json::Value) -> Result<(), DpopError> {
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};
    let crv = jwk.get("crv").and_then(|v| v.as_str()).unwrap_or("");
    if crv != "P-256" {
        return Err(DpopError::UnsupportedAlg(format!("EC crv={crv}")));
    }
    let x = jwk_b64_decode(jwk, "x")?;
    let y = jwk_b64_decode(jwk, "y")?;
    if x.len() != 32 || y.len() != 32 {
        return Err(DpopError::Proof("EC P-256 coords must be 32 bytes"));
    }
    // SEC1 uncompressed point = 0x04 || X || Y.
    let mut spki = Vec::with_capacity(65);
    spki.push(0x04);
    spki.extend_from_slice(&x);
    spki.extend_from_slice(&y);
    if sig.len() != 64 {
        return Err(DpopError::Proof("ES256 signature must be 64 bytes (r||s)"));
    }
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &spki)
        .verify(msg.as_bytes(), sig)
        .map_err(|_| DpopError::BadSignature)
}

fn verify_rs256(msg: &str, sig: &[u8], jwk: &serde_json::Value) -> Result<(), DpopError> {
    use ring::signature::{RsaPublicKeyComponents, RSA_PKCS1_2048_8192_SHA256};
    let n = jwk_b64_decode(jwk, "n")?;
    let e = jwk_b64_decode(jwk, "e")?;
    let comp = RsaPublicKeyComponents { n: &n, e: &e };
    comp.verify(&RSA_PKCS1_2048_8192_SHA256, msg.as_bytes(), sig)
        .map_err(|_| DpopError::BadSignature)
}

fn verify_eddsa(msg: &str, sig: &[u8], jwk: &serde_json::Value) -> Result<(), DpopError> {
    use ring::signature::{UnparsedPublicKey, ED25519};
    let crv = jwk.get("crv").and_then(|v| v.as_str()).unwrap_or("");
    if crv != "Ed25519" {
        return Err(DpopError::UnsupportedAlg(format!("OKP crv={crv}")));
    }
    let x = jwk_b64_decode(jwk, "x")?;
    UnparsedPublicKey::new(&ED25519, &x)
        .verify(msg.as_bytes(), sig)
        .map_err(|_| DpopError::BadSignature)
}

fn jwk_b64_decode(jwk: &serde_json::Value, field: &str) -> Result<Vec<u8>, DpopError> {
    let s = jwk.get(field).and_then(|v| v.as_str()).ok_or(DpopError::Proof("missing JWK field"))?;
    B64.decode(s.as_bytes()).map_err(|e| DpopError::Base64(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{
        EcdsaKeyPair, Ed25519KeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING,
    };
    use std::time::Duration;

    fn make_proof(
        alg: &str,
        sign: impl FnOnce(&[u8]) -> Vec<u8>,
        jwk: serde_json::Value,
        claims: serde_json::Value,
    ) -> String {
        let hdr = serde_json::json!({"typ":"dpop+jwt","alg":alg,"jwk":jwk});
        let hb = B64.encode(serde_json::to_vec(&hdr).unwrap());
        let pb = B64.encode(serde_json::to_vec(&claims).unwrap());
        let input = format!("{hb}.{pb}");
        let sig = sign(input.as_bytes());
        let sb = B64.encode(sig);
        format!("{hb}.{pb}.{sb}")
    }

    fn ed25519_pair() -> (Ed25519KeyPair, serde_json::Value) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let x = B64.encode(kp.public_key().as_ref());
        (kp, serde_json::json!({"kty":"OKP","crv":"Ed25519","x":x}))
    }

    fn p256_pair() -> (EcdsaKeyPair, serde_json::Value) {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let kp = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8.as_ref(),
            &rng,
        )
        .unwrap();
        // SEC1 uncompressed: 0x04 || X || Y
        let pk_bytes = kp.public_key().as_ref().to_vec();
        let x = B64.encode(&pk_bytes[1..33]);
        let y = B64.encode(&pk_bytes[33..65]);
        (kp, serde_json::json!({"kty":"EC","crv":"P-256","x":x,"y":y}))
    }

    // upstream: rfc9449 §4.3 — a well-formed EdDSA proof verifies and yields
    // a `jkt` thumbprint.
    #[test]
    fn eddsa_proof_verifies_and_returns_jkt() {
        let (kp, jwk) = ed25519_pair();
        let claims = serde_json::json!({
            "jti":"unique-jti-1",
            "htm":"POST",
            "htu":"https://srv.example/token",
            "iat": 1_700_000_000
        });
        let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk.clone(), claims);
        let store = NonceStore::new(Duration::from_secs(60));
        let opts = VerifyOptions {
            expected_htm: "POST".into(),
            expected_htu: "https://srv.example/token".into(),
            iat_skew_secs: 60,
            expected_ath: None,
            expected_nonce: None,
            now_unix: 1_700_000_000,
        };
        let v = verify(&proof, &opts, &store).unwrap();
        assert_eq!(v.jkt, jkt(&jwk).unwrap());
        assert_eq!(v.claims.htm, "POST");
    }

    // upstream: rfc9449 §4.3 — a P-256 ES256 proof verifies.
    #[test]
    fn es256_proof_verifies() {
        let (kp, jwk) = p256_pair();
        let claims = serde_json::json!({
            "jti":"es256-jti",
            "htm":"GET",
            "htu":"https://srv.example/resource",
            "iat": 1_700_000_000
        });
        let rng = SystemRandom::new();
        let proof = make_proof("ES256", |m| kp.sign(&rng, m).unwrap().as_ref().to_vec(), jwk, claims);
        let store = NonceStore::new(Duration::from_secs(60));
        let opts = VerifyOptions {
            expected_htm: "GET".into(),
            expected_htu: "https://srv.example/resource".into(),
            iat_skew_secs: 60,
            expected_ath: None,
            expected_nonce: None,
            now_unix: 1_700_000_000,
        };
        verify(&proof, &opts, &store).expect("verify ok");
    }

    // upstream: rfc9449 §4.3 — htm mismatch is rejected.
    #[test]
    fn htm_mismatch_rejected() {
        let (kp, jwk) = ed25519_pair();
        let claims = serde_json::json!({
            "jti":"x","htm":"POST","htu":"https://srv/token","iat":1_700_000_000
        });
        let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk, claims);
        let store = NonceStore::new(Duration::from_secs(60));
        let err = verify(
            &proof,
            &VerifyOptions {
                expected_htm: "GET".into(),
                expected_htu: "https://srv/token".into(),
                iat_skew_secs: 60,
                expected_ath: None,
                expected_nonce: None,
                now_unix: 1_700_000_000,
            },
            &store,
        )
        .unwrap_err();
        assert!(matches!(err, DpopError::HtmMismatch { .. }));
    }

    // upstream: rfc9449 §4.3 — iat outside window rejected.
    #[test]
    fn iat_out_of_window_rejected() {
        let (kp, jwk) = ed25519_pair();
        let claims = serde_json::json!({
            "jti":"x","htm":"POST","htu":"https://srv/token","iat":1_000_000
        });
        let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk, claims);
        let store = NonceStore::new(Duration::from_secs(60));
        let err = verify(
            &proof,
            &VerifyOptions {
                expected_htm: "POST".into(),
                expected_htu: "https://srv/token".into(),
                iat_skew_secs: 60,
                expected_ath: None,
                expected_nonce: None,
                now_unix: 1_700_000_000,
            },
            &store,
        )
        .unwrap_err();
        assert!(matches!(err, DpopError::IatOutOfWindow));
    }

    // upstream: rfc9449 §11.1 — replayed jti rejected.
    #[test]
    fn replayed_jti_rejected() {
        let (kp, jwk) = ed25519_pair();
        let store = NonceStore::new(Duration::from_secs(60));
        let opts = VerifyOptions {
            expected_htm: "POST".into(),
            expected_htu: "https://srv/token".into(),
            iat_skew_secs: 60,
            expected_ath: None,
            expected_nonce: None,
            now_unix: 1_700_000_000,
        };
        for i in 0..2 {
            let claims = serde_json::json!({
                "jti":"same-jti",
                "htm":"POST",
                "htu":"https://srv/token",
                "iat": 1_700_000_000 + i,
            });
            let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk.clone(), claims);
            let r = verify(&proof, &opts, &store);
            if i == 0 {
                r.unwrap();
            } else {
                assert!(matches!(r.unwrap_err(), DpopError::Replay));
            }
        }
    }

    // upstream: rfc9449 §4.3 — signature tampered: bad signature rejected.
    #[test]
    fn tampered_signature_rejected() {
        let (kp, jwk) = ed25519_pair();
        let claims = serde_json::json!({
            "jti":"x","htm":"POST","htu":"https://srv/token","iat":1_700_000_000
        });
        let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk, claims);
        // Flip a byte in the signature segment.
        let mut parts: Vec<&str> = proof.split('.').collect();
        let mut sig = B64.decode(parts[2].as_bytes()).unwrap();
        sig[0] ^= 0x01;
        let new_sig_b64 = B64.encode(&sig);
        parts[2] = &new_sig_b64;
        let tampered = parts.join(".");
        let store = NonceStore::new(Duration::from_secs(60));
        let err = verify(
            &tampered,
            &VerifyOptions {
                expected_htm: "POST".into(),
                expected_htu: "https://srv/token".into(),
                iat_skew_secs: 60,
                expected_ath: None,
                expected_nonce: None,
                now_unix: 1_700_000_000,
            },
            &store,
        )
        .unwrap_err();
        assert_eq!(err, DpopError::BadSignature);
    }

    // upstream: rfc9449 §6 — ath = base64url(SHA-256(access_token)).
    #[test]
    fn ath_helper_matches_rfc9449() {
        // RFC 9449 §6 example access token + expected ath.
        let access_token =
            "Kz~8mXK1EalYznwH-LC-1fBAo.4Ljp~zsPE_NeO.gxU";
        let expected = "fUHyO2r2Z3DZ53EsNrWBb0xWXoaNy59IiKCAqksmQEo";
        assert_eq!(ath_from_access_token(access_token), expected);
    }

    // upstream: rfc9449 §6 — when ath is expected, mismatch is rejected.
    #[test]
    fn ath_mismatch_rejected() {
        let (kp, jwk) = ed25519_pair();
        let claims = serde_json::json!({
            "jti":"ath-jti","htm":"GET","htu":"https://srv/r","iat":1_700_000_000,
            "ath":"wrong-ath"
        });
        let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk, claims);
        let store = NonceStore::new(Duration::from_secs(60));
        let err = verify(
            &proof,
            &VerifyOptions {
                expected_htm: "GET".into(),
                expected_htu: "https://srv/r".into(),
                iat_skew_secs: 60,
                expected_ath: Some("right-ath".into()),
                expected_nonce: None,
                now_unix: 1_700_000_000,
            },
            &store,
        )
        .unwrap_err();
        assert_eq!(err, DpopError::AthMismatch);
    }

    // upstream: rfc9449 §8 — when a nonce is expected, mismatch is rejected.
    #[test]
    fn nonce_mismatch_rejected() {
        let (kp, jwk) = ed25519_pair();
        let claims = serde_json::json!({
            "jti":"n-jti","htm":"POST","htu":"https://srv/token","iat":1_700_000_000,
            "nonce":"client-sent"
        });
        let proof = make_proof("EdDSA", |m| kp.sign(m).as_ref().to_vec(), jwk, claims);
        let store = NonceStore::new(Duration::from_secs(60));
        let err = verify(
            &proof,
            &VerifyOptions {
                expected_htm: "POST".into(),
                expected_htu: "https://srv/token".into(),
                iat_skew_secs: 60,
                expected_ath: None,
                expected_nonce: Some("server-issued".into()),
                now_unix: 1_700_000_000,
            },
            &store,
        )
        .unwrap_err();
        assert_eq!(err, DpopError::NonceMismatch);
    }

    // upstream: rfc9449 §4 — typ must be "dpop+jwt"; otherwise rejected.
    #[test]
    fn wrong_typ_rejected() {
        let (kp, jwk) = ed25519_pair();
        // Hand-craft a proof with typ=JWT instead of dpop+jwt.
        let hdr = serde_json::json!({"typ":"JWT","alg":"EdDSA","jwk":jwk});
        let pl = serde_json::json!({
            "jti":"x","htm":"POST","htu":"https://srv/token","iat":1_700_000_000
        });
        let hb = B64.encode(serde_json::to_vec(&hdr).unwrap());
        let pb = B64.encode(serde_json::to_vec(&pl).unwrap());
        let input = format!("{hb}.{pb}");
        let sig = kp.sign(input.as_bytes());
        let sb = B64.encode(sig.as_ref());
        let proof = format!("{hb}.{pb}.{sb}");

        let store = NonceStore::new(Duration::from_secs(60));
        let err = verify(
            &proof,
            &VerifyOptions {
                expected_htm: "POST".into(),
                expected_htu: "https://srv/token".into(),
                iat_skew_secs: 60,
                expected_ath: None,
                expected_nonce: None,
                now_unix: 1_700_000_000,
            },
            &store,
        )
        .unwrap_err();
        match err {
            DpopError::Proof(_) => {}
            other => panic!("expected Proof error, got {other:?}"),
        }
    }
}
