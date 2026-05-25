// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/encryption/+ RFC 7516 §3 (compact) / §7.2 (JSON)
//
//! JWE compact + JSON serialization.
//!
//! Compact: `BASE64URL(UTF8(HEADER)).BASE64URL(EncryptedKey).BASE64URL(IV).BASE64URL(Ciphertext).BASE64URL(AuthTag)`
//! JSON (general): one-recipient object with the same fields.

use base64::Engine;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::header::{JweHeader, JweHeaderError};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JweCompact {
    pub header: JweHeader,
    pub encrypted_key: Vec<u8>,
    pub iv: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub auth_tag: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JweError {
    #[error("compact JWE must have 5 segments, found {0}")]
    Segments(usize),
    #[error("segment {segment} not base64url: {detail}")]
    Base64 {
        segment: &'static str,
        detail: String,
    },
    #[error("header error: {0}")]
    Header(#[from] JweHeaderError),
    #[error("JSON serialization failed: {0}")]
    Json(String),
}

pub fn compact_encode(jwe: &JweCompact) -> String {
    let h = b64u(jwe.header.to_json().as_bytes());
    let k = b64u(&jwe.encrypted_key);
    let i = b64u(&jwe.iv);
    let c = b64u(&jwe.ciphertext);
    let t = b64u(&jwe.auth_tag);
    format!("{h}.{k}.{i}.{c}.{t}")
}

pub fn compact_decode(s: &str) -> Result<JweCompact, JweError> {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 5 {
        return Err(JweError::Segments(parts.len()));
    }
    let header_bytes = b64u_decode("header", parts[0])?;
    let header = JweHeader::from_json(&header_bytes)?;
    Ok(JweCompact {
        header,
        encrypted_key: b64u_decode("encrypted_key", parts[1])?,
        iv: b64u_decode("iv", parts[2])?,
        ciphertext: b64u_decode("ciphertext", parts[3])?,
        auth_tag: b64u_decode("auth_tag", parts[4])?,
    })
}

/// The protected header in base64url form — this is the AAD per RFC 7516 §5.1
/// step 14.
pub fn aad_for(header: &JweHeader) -> Vec<u8> {
    b64u(header.to_json().as_bytes()).into_bytes()
}

// ── JSON Serialization ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JweJson {
    pub protected: String,
    pub encrypted_key: String,
    pub iv: String,
    pub ciphertext: String,
    pub tag: String,
}

pub fn json_encode(jwe: &JweCompact) -> Result<String, JweError> {
    let obj = JweJson {
        protected: b64u(jwe.header.to_json().as_bytes()),
        encrypted_key: b64u(&jwe.encrypted_key),
        iv: b64u(&jwe.iv),
        ciphertext: b64u(&jwe.ciphertext),
        tag: b64u(&jwe.auth_tag),
    };
    serde_json::to_string(&obj).map_err(|e| JweError::Json(e.to_string()))
}

pub fn json_decode(s: &str) -> Result<JweCompact, JweError> {
    let obj: JweJson = serde_json::from_str(s).map_err(|e| JweError::Json(e.to_string()))?;
    let header_bytes = b64u_decode("header", &obj.protected)?;
    let header = JweHeader::from_json(&header_bytes)?;
    Ok(JweCompact {
        header,
        encrypted_key: b64u_decode("encrypted_key", &obj.encrypted_key)?,
        iv: b64u_decode("iv", &obj.iv)?,
        ciphertext: b64u_decode("ciphertext", &obj.ciphertext)?,
        auth_tag: b64u_decode("auth_tag", &obj.tag)?,
    })
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn b64u(b: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn b64u_decode(segment: &'static str, s: &str) -> Result<Vec<u8>, JweError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|e| JweError::Base64 {
            segment,
            detail: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::super::content_encryption::ContentEncAlg;
    use super::super::header::KeyAgreementAlg;
    use super::*;

    fn sample() -> JweCompact {
        JweCompact {
            header: JweHeader::new(KeyAgreementAlg::RsaOaep, ContentEncAlg::A256Gcm).with_kid("k1"),
            encrypted_key: vec![1, 2, 3, 4],
            iv: vec![5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            ciphertext: b"ciphertext-blob".to_vec(),
            auth_tag: vec![0xAA; 16],
        }
    }

    #[test]
    fn compact_round_trip() {
        let j = sample();
        let s = compact_encode(&j);
        let parts: Vec<&str> = s.split('.').collect();
        assert_eq!(parts.len(), 5);
        let back = compact_decode(&s).unwrap();
        assert_eq!(back, j);
    }

    #[test]
    fn compact_rejects_wrong_segment_count() {
        let err = compact_decode("only.two").unwrap_err();
        assert!(matches!(err, JweError::Segments(2)));
    }

    #[test]
    fn compact_rejects_bad_base64() {
        let err = compact_decode("!!!.!!!.!!!.!!!.!!!").unwrap_err();
        assert!(matches!(
            err,
            JweError::Base64 {
                segment: "header",
                ..
            }
        ));
    }

    #[test]
    fn json_round_trip() {
        let j = sample();
        let s = json_encode(&j).unwrap();
        let back = json_decode(&s).unwrap();
        assert_eq!(back, j);
    }

    #[test]
    fn aad_matches_header_b64() {
        let h = JweHeader::new(KeyAgreementAlg::RsaOaep, ContentEncAlg::A256Gcm);
        let aad = aad_for(&h);
        let expect = b64u(h.to_json().as_bytes()).into_bytes();
        assert_eq!(aad, expect);
    }

    #[test]
    fn json_has_required_members() {
        let s = json_encode(&sample()).unwrap();
        for k in ["protected", "encrypted_key", "iv", "ciphertext", "tag"] {
            assert!(s.contains(k), "missing field {k} in {s}");
        }
    }

    #[test]
    fn json_rejects_invalid() {
        let err = json_decode("not json").unwrap_err();
        assert!(matches!(err, JweError::Json(_)));
    }
}
