// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cosign bundle format — `.sig` + `.pem` + `.crt` + rekor entry.
//!
//! Maps to:
//!   * pkg/cosign/bundle/bundle.go      → CosignBundle
//!   * pkg/cosign/bundle/rekor.go       → RekorBundle
//!   * cmd/cosign/cli/bundle            → bundle sub-command

use crate::error::{Result, SignError};
use crate::models::SigKind;
use serde::{Deserialize, Serialize};

/// The "old" cosign bundle envelope — the JSON that ships next to a
/// `.sig` file. Sigstore protobuf bundle (v0.3) is **not** included in
/// MVP; we accept and emit this envelope which both cosign 2.x and 3.x
/// consume on `--bundle`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CosignBundle {
    pub kind: SigKind,
    /// Base64-encoded signature bytes.
    pub signed_payload_b64: String,
    /// PEM-encoded signing certificate (keyless) or public key (keypair).
    pub cert_pem: String,
    /// PEM-encoded chain (keyless only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_pem: Option<String>,
    /// Rekor log entry pieces — flat fields for easy parsing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rekor_log_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rekor_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rekor_integrated_time: Option<i64>,
    /// Digest the signature covers (artifact digest), `sha256:<hex>`.
    pub artifact_digest: String,
}

impl CosignBundle {
    pub fn encode_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| SignError::Bundle(format!("encode: {}", e)))
    }

    pub fn decode_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| SignError::Bundle(format!("decode: {}", e)))
    }

    pub fn has_rekor_entry(&self) -> bool {
        self.rekor_log_index.is_some()
    }
}

/// Cosign "triple" — for callers that want to write three side-car files
/// (`<artifact>.sig`, `<artifact>.crt`, `<artifact>.bundle`) instead of
/// the JSON bundle. cosign 2.x defaults to this on `cosign sign-blob`.
pub struct BundleTriple {
    pub sig_b64: String,
    pub cert_pem: String,
    pub bundle_json: String,
}

impl BundleTriple {
    pub fn from_bundle(b: &CosignBundle) -> Result<Self> {
        Ok(Self {
            sig_b64: b.signed_payload_b64.clone(),
            cert_pem: b.cert_pem.clone(),
            bundle_json: b.encode_json()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CosignBundle {
        CosignBundle {
            kind: SigKind::Keyless,
            signed_payload_b64: "ZmFrZQ==".into(),
            cert_pem: "-----BEGIN CERTIFICATE-----\nM\n-----END CERTIFICATE-----".into(),
            chain_pem: Some("-----BEGIN CERTIFICATE-----\nC\n-----END CERTIFICATE-----".into()),
            rekor_log_index: Some(42),
            rekor_uuid: Some("deadbeef".into()),
            rekor_integrated_time: Some(1_700_000_042),
            artifact_digest:
                "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".into(),
        }
    }

    #[test]
    fn json_roundtrip() {
        let b = fixture();
        let s = b.encode_json().unwrap();
        let back = CosignBundle::decode_json(&s).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn json_skips_none_fields() {
        let mut b = fixture();
        b.chain_pem = None;
        b.rekor_log_index = None;
        b.rekor_uuid = None;
        b.rekor_integrated_time = None;
        let s = b.encode_json().unwrap();
        assert!(!s.contains("chain_pem"));
        assert!(!s.contains("rekor_log_index"));
    }

    #[test]
    fn has_rekor_entry_detection() {
        let b = fixture();
        assert!(b.has_rekor_entry());
        let mut b2 = fixture();
        b2.rekor_log_index = None;
        assert!(!b2.has_rekor_entry());
    }

    #[test]
    fn triple_carries_pieces() {
        let b = fixture();
        let t = BundleTriple::from_bundle(&b).unwrap();
        assert_eq!(t.sig_b64, b.signed_payload_b64);
        assert!(t.cert_pem.contains("CERTIFICATE"));
        assert!(t.bundle_json.contains("\"kind\""));
    }

    #[test]
    fn rejects_invalid_json() {
        let err = CosignBundle::decode_json("{").expect_err("must fail");
        assert!(matches!(err, SignError::Bundle(_)));
    }

    #[test]
    fn keypair_kind_serializes() {
        let mut b = fixture();
        b.kind = SigKind::Keypair;
        b.chain_pem = None;
        let s = b.encode_json().unwrap();
        assert!(s.contains("\"keypair\""));
    }

    #[test]
    fn rekor_fields_optional_in_decode() {
        let raw = r#"{"kind":"keypair","signed_payload_b64":"x","cert_pem":"p","artifact_digest":"sha256:00"}"#;
        let b = CosignBundle::decode_json(raw).unwrap();
        assert!(!b.has_rekor_entry());
        assert!(b.chain_pem.is_none());
    }
}
