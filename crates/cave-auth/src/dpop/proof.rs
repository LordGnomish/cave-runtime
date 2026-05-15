// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/grants/+ RFC 9449 §4 (DPoP proof JWT)
//
//! DPoP proof JWT parser — RFC 9449 §4.
//!
//! A DPoP proof is a JWT whose header MUST carry:
//!   - `typ = "dpop+jwt"`
//!   - `alg` one of the asymmetric algorithms listed in [`SUPPORTED_ALGS`]
//!   - `jwk` — the public key matching the proof signature
//!
//! And whose payload MUST carry:
//!   - `jti` — unique identifier
//!   - `htm` — HTTP method
//!   - `htu` — HTTP URI (no fragment, no query usually but we accept query)
//!   - `iat` — issued-at time
//!
//! Optional:
//!   - `ath` — base64url SHA-256 of access-token bytes (when a DPoP-bound access token is in use)
//!   - `nonce` — server-issued nonce
//!
//! This module is the **parse-only** layer. Signature verification & header
//! semantic checks against the request are in [`super::verify`].

use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::binding::Jwk;

/// Algorithms accepted by RFC 9449 §4.2 — we restrict to the two asymmetric
/// signature schemes the wider DPoP ecosystem actually deploys.
pub const SUPPORTED_ALGS: &[&str] = &["ES256", "RS256"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DpopHeader {
    pub alg: String,
    pub typ: String,
    pub jwk: Jwk,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DpopPayload {
    pub jti: String,
    pub htm: String,
    pub htu: String,
    pub iat: i64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub nonce: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DpopProof {
    pub header: DpopHeader,
    pub payload: DpopPayload,
    /// Raw signing input (`header.payload`) — needed for downstream signature verification.
    pub signing_input: String,
    /// Raw signature bytes (base64url-decoded).
    pub signature: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DpopProofError {
    #[error("DPoP proof must have 3 dot-separated segments, found {0}")]
    MalformedSegments(usize),
    #[error("DPoP proof segment {segment} is not valid base64url: {detail}")]
    Base64 { segment: &'static str, detail: String },
    #[error("DPoP proof segment {segment} is not valid JSON: {detail}")]
    Json { segment: &'static str, detail: String },
    #[error("DPoP proof header typ MUST be \"dpop+jwt\", found {0:?}")]
    BadTyp(String),
    #[error("DPoP proof header alg {0:?} is not in the allowed list {1:?}")]
    UnsupportedAlg(String, &'static [&'static str]),
}

impl DpopProof {
    /// Parses a `DPoP` header value into typed components.
    ///
    /// Per RFC 9449 §4.3, the receiver MUST verify these structural rules
    /// **before** attempting cryptographic verification.
    pub fn parse(jwt: &str) -> Result<Self, DpopProofError> {
        let parts: Vec<&str> = jwt.split('.').collect();
        if parts.len() != 3 {
            return Err(DpopProofError::MalformedSegments(parts.len()));
        }
        let header_bytes = b64url_decode("header", parts[0])?;
        let payload_bytes = b64url_decode("payload", parts[1])?;
        let signature = b64url_decode("signature", parts[2])?;

        let header: DpopHeader = serde_json::from_slice(&header_bytes).map_err(|e| {
            DpopProofError::Json {
                segment: "header",
                detail: e.to_string(),
            }
        })?;
        let payload: DpopPayload = serde_json::from_slice(&payload_bytes).map_err(|e| {
            DpopProofError::Json {
                segment: "payload",
                detail: e.to_string(),
            }
        })?;

        if header.typ != "dpop+jwt" {
            return Err(DpopProofError::BadTyp(header.typ.clone()));
        }
        if !SUPPORTED_ALGS.contains(&header.alg.as_str()) {
            return Err(DpopProofError::UnsupportedAlg(
                header.alg.clone(),
                SUPPORTED_ALGS,
            ));
        }

        let signing_input = format!("{}.{}", parts[0], parts[1]);
        Ok(DpopProof {
            header,
            payload,
            signing_input,
            signature,
        })
    }
}

fn b64url_decode(segment: &'static str, s: &str) -> Result<Vec<u8>, DpopProofError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|e| DpopProofError::Base64 {
            segment,
            detail: e.to_string(),
        })
}

/// Helper for tests + the in-tree signer (used by `cavectl auth dpop make-proof`).
pub fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_part(json: &str) -> String {
        b64url_encode(json.as_bytes())
    }

    fn sample_proof(typ: &str, alg: &str) -> String {
        let h = format!(
            r#"{{"alg":"{alg}","typ":"{typ}","jwk":{{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}}}}"#
        );
        let p = r#"{"jti":"abc","htm":"POST","htu":"https://server.example/token","iat":1700000000}"#;
        let sig = b64url_encode(&[0u8; 64]);
        format!("{}.{}.{}", encode_part(&h), encode_part(p), sig)
    }

    #[test]
    fn parse_valid_dpop_proof() {
        let proof = DpopProof::parse(&sample_proof("dpop+jwt", "ES256")).unwrap();
        assert_eq!(proof.header.alg, "ES256");
        assert_eq!(proof.header.typ, "dpop+jwt");
        assert_eq!(proof.payload.jti, "abc");
        assert_eq!(proof.payload.htm, "POST");
        assert_eq!(proof.payload.iat, 1700000000);
    }

    #[test]
    fn reject_wrong_typ() {
        let err = DpopProof::parse(&sample_proof("JWT", "ES256")).unwrap_err();
        assert!(matches!(err, DpopProofError::BadTyp(t) if t == "JWT"));
    }

    #[test]
    fn reject_unsupported_alg() {
        let err = DpopProof::parse(&sample_proof("dpop+jwt", "HS256")).unwrap_err();
        assert!(matches!(err, DpopProofError::UnsupportedAlg(a, _) if a == "HS256"));
    }

    #[test]
    fn reject_malformed_segments() {
        let err = DpopProof::parse("only.two").unwrap_err();
        assert!(matches!(err, DpopProofError::MalformedSegments(2)));
    }

    #[test]
    fn reject_invalid_base64() {
        let err = DpopProof::parse("!!!.payload.sig").unwrap_err();
        assert!(matches!(err, DpopProofError::Base64 { segment: "header", .. }));
    }

    #[test]
    fn signing_input_is_first_two_segments() {
        let raw = sample_proof("dpop+jwt", "ES256");
        let proof = DpopProof::parse(&raw).unwrap();
        let expect_prefix = raw.rsplit_once('.').unwrap().0;
        assert_eq!(proof.signing_input, expect_prefix);
    }

    #[test]
    fn signature_decodes_to_bytes() {
        let proof = DpopProof::parse(&sample_proof("dpop+jwt", "ES256")).unwrap();
        assert_eq!(proof.signature.len(), 64); // we encoded 64 zero bytes
    }

    #[test]
    fn optional_ath_is_parsed_when_present() {
        let h = r#"{"alg":"ES256","typ":"dpop+jwt","jwk":{"kty":"EC","crv":"P-256","x":"AAAA","y":"BBBB"}}"#;
        let p = r#"{"jti":"x","htm":"GET","htu":"https://r","iat":1,"ath":"abc"}"#;
        let raw = format!(
            "{}.{}.{}",
            encode_part(h),
            encode_part(p),
            b64url_encode(&[0u8; 64])
        );
        let proof = DpopProof::parse(&raw).unwrap();
        assert_eq!(proof.payload.ath.as_deref(), Some("abc"));
    }

    #[test]
    fn rs256_is_supported() {
        let proof = DpopProof::parse(&sample_proof("dpop+jwt", "RS256")).unwrap();
        assert_eq!(proof.header.alg, "RS256");
    }
}
