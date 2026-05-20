// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: sigstore/cosign@HEAD pkg/oci/signature.go + specs/SIGNATURE_SPEC.md
//! OCI signature manifest attachment.
//!
//! Cosign attaches signatures to a subject manifest by:
//!   1. Computing the subject's `sha256:XYZ` digest.
//!   2. Building a "simple signing" payload JSON (Cosign payload format)
//!      that pins that digest under `critical.image.docker-manifest-digest`.
//!   3. Signing the payload bytes with the chosen algorithm.
//!   4. Pushing a sibling artifact tagged `sha256-XYZ.sig` whose layers
//!      are `(payload, signature)` pairs.
//!
//! This module owns step 2 — payload construction and digest binding —
//! and the in-memory signature index that maps digest → signatures (for
//! verification flows that don't actually want to hit a registry).

use super::{CosignError, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::RwLock;

/// Cosign "simple signing" payload, cite: sigstore/cosign payload schema.
///
/// `critical.identity` is intentionally free-form — Cosign uses it to
/// carry image-reference identity attestations; we mirror the shape so
/// existing Cosign consumers can validate our signatures by digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CosignPayload {
    pub critical: CriticalSection,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub optional: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticalSection {
    pub identity: Identity,
    pub image: ImageRef,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    #[serde(rename = "docker-reference")]
    pub docker_reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRef {
    #[serde(rename = "docker-manifest-digest")]
    pub docker_manifest_digest: String,
}

/// Compute the canonical Cosign payload bytes for a given image
/// reference + digest. This is what gets fed into `cosign::sign`.
pub fn build_payload(reference: &str, digest: &str) -> Vec<u8> {
    let payload = CosignPayload {
        critical: CriticalSection {
            identity: Identity {
                docker_reference: reference.to_string(),
            },
            image: ImageRef {
                docker_manifest_digest: digest.to_string(),
            },
            kind: "cosign container image signature".into(),
        },
        optional: HashMap::new(),
    };
    serde_json::to_vec(&payload).expect("payload always serializes")
}

/// Convenience: SHA-256 of `bytes` formatted as `sha256:HEX`. Used to
/// produce the digest that gets pinned inside the payload.
pub fn manifest_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// Compute the OCI tag Cosign would push the signature artifact under.
/// Cite: sigstore/cosign README — `sha256-XYZ.sig` for image signatures.
pub fn signature_tag(digest: &str) -> Result<String, CosignError> {
    let stripped = digest.strip_prefix("sha256:").ok_or_else(|| {
        CosignError::PayloadInvalid(format!("digest must be sha256:HEX, got {digest}"))
    })?;
    Ok(format!("sha256-{stripped}.sig"))
}

/// In-memory store mapping digest → list of (payload, signature) pairs.
/// Mirrors the per-tag layer shape Cosign would push to a registry but
/// avoids the round-trip for tests / local-only verification.
#[derive(Default)]
pub struct SignatureIndex {
    inner: RwLock<HashMap<String, Vec<SignatureRecord>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureRecord {
    pub payload_b64: String,
    pub signature: Signature,
}

impl SignatureIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(&self, digest: &str, payload: &[u8], signature: Signature) {
        use base64::Engine as _;
        use base64::engine::general_purpose::STANDARD;
        let rec = SignatureRecord {
            payload_b64: STANDARD.encode(payload),
            signature,
        };
        self.inner
            .write()
            .unwrap()
            .entry(digest.to_string())
            .or_default()
            .push(rec);
    }

    pub fn list(&self, digest: &str) -> Vec<SignatureRecord> {
        self.inner
            .read()
            .unwrap()
            .get(digest)
            .cloned()
            .unwrap_or_default()
    }

    pub fn count(&self, digest: &str) -> usize {
        self.inner
            .read()
            .unwrap()
            .get(digest)
            .map(|v| v.len())
            .unwrap_or(0)
    }

    pub fn remove(&self, digest: &str) -> usize {
        self.inner
            .write()
            .unwrap()
            .remove(digest)
            .map(|v| v.len())
            .unwrap_or(0)
    }
}
