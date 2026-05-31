// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sigstore protobuf bundle v0.3 — the modern, self-describing envelope.
//!
//! Maps to:
//!   * sigstore/protobuf-specs `dev.sigstore.bundle.v1.Bundle`
//!   * pkg/cosign/bundle/protobuf_bundle (cosign `--new-bundle-format`)
//!   * sigstore-go `bundle.Bundle`
//!
//! cosign v3 emits this envelope by default. It supersedes the flat
//! `CosignBundle` JSON (still produced for 2.x consumers) by self-describing
//! its `mediaType`, carrying the verification material (cert *or* public-key
//! hint + Rekor tlog entries) and either a `messageSignature` (blob/image)
//! or a `dsseEnvelope` (attestation). This module owns the *serialization*:
//! protobuf-JSON (protojson) wire shape — camelCase fields, base64 bytes,
//! and int64 fields rendered as JSON **strings** — derived from the
//! [`crate::bundle::CosignBundle`] we already build elsewhere.

use crate::bundle::CosignBundle;
use crate::error::{Result, SignError};
use crate::models::SigKind;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Media type for the v0.3 bundle (protobuf-specs `Bundle` at the JSON layer).
pub const BUNDLE_MEDIA_TYPE_V03: &str = "application/vnd.dev.sigstore.bundle.v0.3+json";

/// `dev.sigstore.bundle.v1.Bundle` rendered as protojson.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigstoreBundle {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    #[serde(rename = "verificationMaterial")]
    pub verification_material: VerificationMaterial,
    #[serde(rename = "messageSignature", skip_serializing_if = "Option::is_none")]
    pub message_signature: Option<MessageSignature>,
    #[serde(rename = "dsseEnvelope", skip_serializing_if = "Option::is_none")]
    pub dsse_envelope: Option<serde_json::Value>,
}

/// `VerificationMaterial` — exactly one of certificate / publicKey identifies
/// the signer, plus the transparency-log entries that bind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationMaterial {
    #[serde(rename = "certificate", skip_serializing_if = "Option::is_none")]
    pub certificate: Option<X509Certificate>,
    #[serde(rename = "publicKey", skip_serializing_if = "Option::is_none")]
    pub public_key: Option<PublicKeyIdentifier>,
    #[serde(rename = "tlogEntries", default, skip_serializing_if = "Vec::is_empty")]
    pub tlog_entries: Vec<TlogEntry>,
}

/// `dev.sigstore.common.v1.X509Certificate` — DER bytes, base64 in protojson.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X509Certificate {
    #[serde(rename = "rawBytes")]
    pub raw_bytes: String,
}

/// `PublicKeyIdentifier` — a hint into the trusted-root key set (keypair mode,
/// where no Fulcio certificate exists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicKeyIdentifier {
    pub hint: String,
}

/// `dev.sigstore.common.v1.MessageSignature`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSignature {
    #[serde(rename = "messageDigest")]
    pub message_digest: MessageDigest,
    /// Base64 raw signature bytes.
    pub signature: String,
}

/// `dev.sigstore.common.v1.HashOutput`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDigest {
    /// Enum name from `HashAlgorithm` — `SHA2_256` for cosign.
    pub algorithm: String,
    /// Base64 raw digest bytes.
    pub digest: String,
}

/// `dev.sigstore.rekor.v1.TransparencyLogEntry` (subset cosign populates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlogEntry {
    /// int64 → protojson string.
    #[serde(rename = "logIndex")]
    pub log_index: String,
    #[serde(rename = "logId")]
    pub log_id: LogId,
    #[serde(rename = "kindVersion")]
    pub kind_version: KindVersion,
    /// int64 → protojson string.
    #[serde(rename = "integratedTime")]
    pub integrated_time: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogId {
    /// Base64 log public-key id (here the Rekor entry UUID stands in).
    #[serde(rename = "keyId")]
    pub key_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindVersion {
    pub kind: String,
    pub version: String,
}

impl SigstoreBundle {
    /// Build a message-signature bundle (blob / OCI image) from the flat
    /// cosign bundle. Mirrors sigstore-go `bundle.NewBundle` for the
    /// `messageSignature` case.
    pub fn from_cosign_bundle(b: &CosignBundle) -> Result<Self> {
        Ok(Self {
            media_type: BUNDLE_MEDIA_TYPE_V03.into(),
            verification_material: verification_material(b)?,
            message_signature: Some(MessageSignature {
                message_digest: MessageDigest {
                    algorithm: "SHA2_256".into(),
                    digest: digest_b64_from_artifact(&b.artifact_digest)?,
                },
                signature: b.signed_payload_b64.clone(),
            }),
            dsse_envelope: None,
        })
    }

    /// Build an attestation bundle carrying the DSSE envelope verbatim.
    pub fn from_dsse(b: &CosignBundle, envelope: serde_json::Value) -> Result<Self> {
        Ok(Self {
            media_type: BUNDLE_MEDIA_TYPE_V03.into(),
            verification_material: verification_material(b)?,
            message_signature: None,
            dsse_envelope: Some(envelope),
        })
    }

    pub fn encode_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| SignError::Bundle(format!("sigstore-bundle encode: {}", e)))
    }

    pub fn decode_json(s: &str) -> Result<Self> {
        serde_json::from_str(s)
            .map_err(|e| SignError::Bundle(format!("sigstore-bundle decode: {}", e)))
    }
}

/// Keyless → X.509 certificate (rawBytes = DER); keypair → publicKey hint.
fn verification_material(b: &CosignBundle) -> Result<VerificationMaterial> {
    let (certificate, public_key) = match b.kind {
        SigKind::Keyless => (
            Some(X509Certificate {
                raw_bytes: pem_body(&b.cert_pem)?,
            }),
            None,
        ),
        SigKind::Keypair => (
            None,
            Some(PublicKeyIdentifier {
                hint: pem_body(&b.cert_pem)?,
            }),
        ),
    };
    let tlog_entries = match (b.rekor_log_index, &b.rekor_uuid, b.rekor_integrated_time) {
        (Some(idx), Some(uuid), Some(t)) => vec![TlogEntry {
            log_index: idx.to_string(),
            log_id: LogId {
                key_id: uuid.clone(),
            },
            kind_version: KindVersion {
                kind: "hashedrekord".into(),
                version: "0.0.1".into(),
            },
            integrated_time: t.to_string(),
        }],
        _ => Vec::new(),
    };
    Ok(VerificationMaterial {
        certificate,
        public_key,
        tlog_entries,
    })
}

/// Strip PEM armor + newlines, returning the base64 body — which for an
/// X.509 PEM *is* the base64 of the DER (`rawBytes` in protobuf-specs).
fn pem_body(pem: &str) -> Result<String> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .flat_map(|l| l.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    if body.is_empty() {
        return Err(SignError::Bundle("empty PEM body".into()));
    }
    Ok(body)
}

/// `sha256:<hex>` → base64 of the raw 32 digest bytes.
fn digest_b64_from_artifact(artifact_digest: &str) -> Result<String> {
    let hex = artifact_digest
        .strip_prefix("sha256:")
        .unwrap_or(artifact_digest);
    let raw = hex::decode(hex)
        .map_err(|e| SignError::Bundle(format!("artifact digest hex: {}", e)))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::CosignBundle;
    use crate::models::SigKind;

    fn keyless_bundle() -> CosignBundle {
        CosignBundle {
            kind: SigKind::Keyless,
            signed_payload_b64: "c2lnbmF0dXJl".into(), // "signature"
            // PEM body decodes (base64) to DER bytes — rawBytes must equal the body.
            cert_pem: "-----BEGIN CERTIFICATE-----\nQUJDREVG\nR0hJSktM\n-----END CERTIFICATE-----"
                .into(),
            chain_pem: None,
            rekor_log_index: Some(42),
            rekor_uuid: Some("deadbeefcafe".into()),
            rekor_integrated_time: Some(1_700_000_042),
            artifact_digest:
                "sha256:abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".into(),
        }
    }

    fn keypair_bundle() -> CosignBundle {
        CosignBundle {
            kind: SigKind::Keypair,
            signed_payload_b64: "c2ln".into(),
            cert_pem: "-----BEGIN PUBLIC KEY-----\nQQ==\n-----END PUBLIC KEY-----".into(),
            chain_pem: None,
            rekor_log_index: None,
            rekor_uuid: None,
            rekor_integrated_time: None,
            artifact_digest:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000".into(),
        }
    }

    #[test]
    fn media_type_is_v03() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        assert_eq!(b.media_type, BUNDLE_MEDIA_TYPE_V03);
    }

    #[test]
    fn keyless_carries_certificate_raw_bytes() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let cert = b.verification_material.certificate.expect("cert present");
        // PEM body (newlines stripped) is the base64 DER == rawBytes.
        assert_eq!(cert.raw_bytes, "QUJDREVGR0hJSktM");
        assert!(b.verification_material.public_key.is_none());
    }

    #[test]
    fn keypair_carries_public_key_not_certificate() {
        let b = SigstoreBundle::from_cosign_bundle(&keypair_bundle()).unwrap();
        assert!(b.verification_material.certificate.is_none());
        assert!(b.verification_material.public_key.is_some());
    }

    #[test]
    fn message_signature_digest_is_sha2_256_base64() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let ms = b.message_signature.expect("message signature present");
        assert_eq!(ms.message_digest.algorithm, "SHA2_256");
        // sha256:abcdef...90 (hex) -> raw 32 bytes -> base64.
        assert_eq!(
            ms.message_digest.digest,
            "q83vEjRWeJCrze8SNFZ4kKvN7xI0VniQq83vEjRWeJA="
        );
        assert_eq!(ms.signature, "c2lnbmF0dXJl");
    }

    #[test]
    fn tlog_entry_encodes_int64_as_string() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        assert_eq!(b.verification_material.tlog_entries.len(), 1);
        let e = &b.verification_material.tlog_entries[0];
        // protojson encodes int64 fields as strings.
        assert_eq!(e.log_index, "42");
        assert_eq!(e.integrated_time, "1700000042");
        assert_eq!(e.kind_version.kind, "hashedrekord");
        assert_eq!(e.kind_version.version, "0.0.1");
    }

    #[test]
    fn no_rekor_means_no_tlog_entries() {
        let b = SigstoreBundle::from_cosign_bundle(&keypair_bundle()).unwrap();
        assert!(b.verification_material.tlog_entries.is_empty());
    }

    #[test]
    fn json_uses_protobuf_camelcase_field_names() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let j = b.encode_json().unwrap();
        assert!(j.contains("\"mediaType\""));
        assert!(j.contains("\"verificationMaterial\""));
        assert!(j.contains("\"messageSignature\""));
        assert!(j.contains("\"messageDigest\""));
        assert!(j.contains("\"tlogEntries\""));
        assert!(j.contains("\"logIndex\""));
        // snake_case must NOT leak.
        assert!(!j.contains("message_signature"));
        assert!(!j.contains("log_index"));
    }

    #[test]
    fn json_roundtrip() {
        let b = SigstoreBundle::from_cosign_bundle(&keyless_bundle()).unwrap();
        let j = b.encode_json().unwrap();
        let back = SigstoreBundle::decode_json(&j).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn dsse_bundle_has_envelope_not_message_signature() {
        let env = serde_json::json!({
            "payload": "eyJfdHlwZSI6Imh0dHBzOi8vaW4tdG90by5pby9TdGF0ZW1lbnQvdjEifQ==",
            "payloadType": "application/vnd.in-toto+json",
            "signatures": [{"sig": "YWJj"}]
        });
        let b = SigstoreBundle::from_dsse(&keyless_bundle(), env.clone()).unwrap();
        assert!(b.message_signature.is_none());
        assert_eq!(b.dsse_envelope, Some(env));
        let j = b.encode_json().unwrap();
        assert!(j.contains("\"dsseEnvelope\""));
        assert!(!j.contains("\"messageSignature\""));
    }
}
