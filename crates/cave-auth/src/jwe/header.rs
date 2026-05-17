// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/.../protocol/oidc/encryption/+ RFC 7516 §4
//
//! JWE protected header (RFC 7516 §4).

use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

use super::content_encryption::ContentEncAlg;

/// Supported key-management algorithms.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeyAgreementAlg {
    #[serde(rename = "RSA-OAEP")]
    RsaOaep,
    #[serde(rename = "ECDH-ES+A256KW")]
    EcdhEsA256Kw,
}

impl KeyAgreementAlg {
    pub fn as_str(&self) -> &'static str {
        match self {
            KeyAgreementAlg::RsaOaep => "RSA-OAEP",
            KeyAgreementAlg::EcdhEsA256Kw => "ECDH-ES+A256KW",
        }
    }
}

impl FromStr for KeyAgreementAlg {
    type Err = JweHeaderError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "RSA-OAEP" => KeyAgreementAlg::RsaOaep,
            "ECDH-ES+A256KW" => KeyAgreementAlg::EcdhEsA256Kw,
            other => return Err(JweHeaderError::UnsupportedAlg(other.to_string())),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JweHeader {
    pub alg: KeyAgreementAlg,
    pub enc: ContentEncAlg,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub kid: Option<String>,
    /// Optional `cty` (content type, e.g. "JWT" for nested).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cty: Option<String>,
    /// Optional ephemeral public key for `ECDH-ES+A256KW`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub epk: Option<EphemeralPublicKey>,
}

/// Just enough of an EC public key for `ECDH-ES+A256KW`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EphemeralPublicKey {
    pub kty: String,
    pub crv: String,
    pub x: String,
    pub y: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum JweHeaderError {
    #[error("unsupported alg {0:?}")]
    UnsupportedAlg(String),
    #[error("unsupported enc {0:?}")]
    UnsupportedEnc(String),
    #[error("header JSON parse failed: {0}")]
    Json(String),
}

impl JweHeader {
    pub fn new(alg: KeyAgreementAlg, enc: ContentEncAlg) -> Self {
        Self {
            alg,
            enc,
            kid: None,
            cty: None,
            epk: None,
        }
    }

    pub fn with_kid(mut self, kid: impl Into<String>) -> Self {
        self.kid = Some(kid.into());
        self
    }

    pub fn with_cty(mut self, cty: impl Into<String>) -> Self {
        self.cty = Some(cty.into());
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("JweHeader is always Serialize")
    }

    pub fn from_json(s: &[u8]) -> Result<Self, JweHeaderError> {
        serde_json::from_slice(s).map_err(|e| JweHeaderError::Json(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_serialises_alg_and_enc() {
        let h = JweHeader::new(KeyAgreementAlg::RsaOaep, ContentEncAlg::A256Gcm);
        let json = h.to_json();
        assert!(json.contains(r#""alg":"RSA-OAEP""#));
        assert!(json.contains(r#""enc":"A256GCM""#));
    }

    #[test]
    fn header_round_trip() {
        let h = JweHeader::new(KeyAgreementAlg::EcdhEsA256Kw, ContentEncAlg::A128CbcHs256)
            .with_kid("key-1")
            .with_cty("JWT");
        let json = h.to_json();
        let back = JweHeader::from_json(json.as_bytes()).unwrap();
        assert_eq!(back, h);
    }

    #[test]
    fn unsupported_alg_rejected() {
        let json = r#"{"alg":"dir","enc":"A256GCM"}"#;
        let err = JweHeader::from_json(json.as_bytes()).unwrap_err();
        assert!(matches!(err, JweHeaderError::Json(_)));
    }

    #[test]
    fn alg_from_str() {
        assert_eq!(
            "RSA-OAEP".parse::<KeyAgreementAlg>().unwrap(),
            KeyAgreementAlg::RsaOaep
        );
        assert_eq!(
            "ECDH-ES+A256KW".parse::<KeyAgreementAlg>().unwrap(),
            KeyAgreementAlg::EcdhEsA256Kw
        );
    }

    #[test]
    fn alg_from_str_unknown() {
        assert!("A256KW".parse::<KeyAgreementAlg>().is_err());
    }

    #[test]
    fn kid_omitted_when_none() {
        let h = JweHeader::new(KeyAgreementAlg::RsaOaep, ContentEncAlg::A256Gcm);
        let json = h.to_json();
        assert!(!json.contains("kid"));
    }

    #[test]
    fn cty_present_for_nested_jwt() {
        let h =
            JweHeader::new(KeyAgreementAlg::RsaOaep, ContentEncAlg::A256Gcm).with_cty("JWT");
        let json = h.to_json();
        assert!(json.contains(r#""cty":"JWT""#));
    }

    #[test]
    fn epk_round_trip() {
        let mut h = JweHeader::new(KeyAgreementAlg::EcdhEsA256Kw, ContentEncAlg::A256Gcm);
        h.epk = Some(EphemeralPublicKey {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: "AAAA".into(),
            y: "BBBB".into(),
        });
        let back = JweHeader::from_json(h.to_json().as_bytes()).unwrap();
        assert_eq!(back, h);
    }
}
