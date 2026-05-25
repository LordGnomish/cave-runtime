// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Core cave-sign models — Sigstore Cosign data primitives.
//!
//! Maps to upstream:
//!   * pkg/cosign/common.go        → SignedArtifact + SigKind
//!   * pkg/cosign/attestation/*    → Attestation + Predicate
//!   * pkg/cosign/bundle/*         → BundleEntry
//!   * cmd/cosign/cli/options/key  → KeyAlgorithm

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Algorithms supported by cave-sign. Cosign's default is ECDSA P-256;
/// Ed25519 is the second-best path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum KeyAlgorithm {
    EcdsaP256,
    Ed25519,
}

impl KeyAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyAlgorithm::EcdsaP256 => "ecdsa-p256",
            KeyAlgorithm::Ed25519 => "ed25519",
        }
    }
}

/// Backwards-compatible record persisted by `engine.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedArtifact {
    pub id: Uuid,
    pub artifact_digest: String,
    pub artifact_type: ArtifactType,
    pub signature: String,
    pub signer_identity: String,
    pub signed_at: DateTime<Utc>,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    ContainerImage,
    Binary,
    Chart,
    Sbom,
    Blob,
    OciArtifact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifyResult {
    pub artifact_digest: String,
    pub valid: bool,
    pub signer: Option<String>,
    pub reason: Option<String>,
}

/// How the signature was produced — keypair (long-lived) or keyless
/// (Fulcio-issued ephemeral cert + OIDC identity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigKind {
    Keypair,
    Keyless,
}

/// Signature material: bytes + the chain that explains who produced them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature {
    pub kind: SigKind,
    /// Base64-encoded raw signature bytes.
    pub sig_b64: String,
    /// PEM-encoded signing certificate (keyless) or public key (keypair).
    pub cert_pem: String,
    /// Optional intermediate + root CA chain (keyless).
    pub chain_pem: Option<String>,
    /// Rekor log entry index (keyless only — present after rekor.upload).
    pub log_index: Option<u64>,
}

/// SLSA / in-toto attestation envelope. Predicate is opaque JSON so the
/// type covers SLSA Provenance + VEX + custom predicates with a single
/// surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attestation {
    /// `application/vnd.in-toto+json`.
    pub media_type: String,
    pub predicate_type: PredicateType,
    pub subject: Vec<Subject>,
    pub predicate: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PredicateType {
    SlsaProvenance,
    Vex,
    Spdx,
    CycloneDx,
    Link,
    Custom(String),
}

impl PredicateType {
    /// URI as emitted by `cosign attest --type`.
    pub fn uri(&self) -> String {
        match self {
            PredicateType::SlsaProvenance => "https://slsa.dev/provenance/v1".into(),
            PredicateType::Vex => "https://openvex.dev/ns/v0.2.0".into(),
            PredicateType::Spdx => "https://spdx.dev/Document".into(),
            PredicateType::CycloneDx => "https://cyclonedx.org/bom".into(),
            PredicateType::Link => "https://in-toto.io/Link/v1".into(),
            PredicateType::Custom(u) => u.clone(),
        }
    }

    pub fn from_uri(uri: &str) -> Self {
        match uri {
            "https://slsa.dev/provenance/v1" => PredicateType::SlsaProvenance,
            "https://openvex.dev/ns/v0.2.0" => PredicateType::Vex,
            "https://spdx.dev/Document" => PredicateType::Spdx,
            "https://cyclonedx.org/bom" => PredicateType::CycloneDx,
            "https://in-toto.io/Link/v1" => PredicateType::Link,
            other => PredicateType::Custom(other.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subject {
    pub name: String,
    /// `{"sha256": "<hex>", ...}`.
    pub digest: std::collections::BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_algorithm_as_str() {
        assert_eq!(KeyAlgorithm::EcdsaP256.as_str(), "ecdsa-p256");
        assert_eq!(KeyAlgorithm::Ed25519.as_str(), "ed25519");
    }

    #[test]
    fn predicate_uri_roundtrip() {
        for p in [
            PredicateType::SlsaProvenance,
            PredicateType::Vex,
            PredicateType::Spdx,
            PredicateType::CycloneDx,
            PredicateType::Link,
            PredicateType::Custom("https://example.com/x".into()),
        ] {
            let u = p.uri();
            let back = PredicateType::from_uri(&u);
            assert_eq!(p, back, "roundtrip mismatch for {:?}", p);
        }
    }

    #[test]
    fn signature_serde() {
        let s = Signature {
            kind: SigKind::Keyless,
            sig_b64: "deadbeef".into(),
            cert_pem: "-----BEGIN CERTIFICATE-----\nM\n-----END CERTIFICATE-----".into(),
            chain_pem: None,
            log_index: Some(42),
        };
        let j = serde_json::to_string(&s).unwrap();
        let back: Signature = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn artifact_type_serde() {
        let j = serde_json::to_string(&ArtifactType::OciArtifact).unwrap();
        assert_eq!(j, "\"oci_artifact\"");
    }

    #[test]
    fn subject_digest_btree() {
        let mut s = Subject {
            name: "ghcr.io/cave/x".into(),
            digest: Default::default(),
        };
        s.digest.insert("sha256".into(), "abc".into());
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["digest"]["sha256"], "abc");
    }
}
