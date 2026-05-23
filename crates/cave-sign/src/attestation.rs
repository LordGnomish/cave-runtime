// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SLSA / in-toto attestation envelope.
//!
//! Maps to:
//!   * pkg/cosign/attestation     → DSSE envelope
//!   * cmd/cosign/cli/attest      → `cosign attest`
//!   * cmd/cosign/cli/attest_blob → `cosign attest-blob`
//!
//! `Attestation` (models.rs) is the wire shape. This module owns the DSSE
//! `Envelope` + the signed in-toto Statement that wraps it, plus the SLSA
//! provenance predicate constructor.

use crate::error::{Result, SignError};
use crate::models::{Attestation, KeyAlgorithm, PredicateType, Subject};
use crate::signature::Keypair;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// DSSE envelope — Sigstore's wire format for signed attestations.
/// One envelope can carry multiple signatures (verification picks any
/// that matches an allowed identity).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DsseEnvelope {
    /// `application/vnd.in-toto+json`.
    pub payload_type: String,
    /// Base64-encoded in-toto Statement JSON.
    pub payload: String,
    pub signatures: Vec<DsseSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DsseSignature {
    /// Key id — usually the certificate fingerprint or a key handle.
    pub keyid: String,
    /// Base64-encoded raw signature bytes.
    pub sig: String,
}

/// `in-toto Statement v1` — the unwrapped, unsigned attestation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InTotoStatement {
    #[serde(rename = "_type")]
    pub statement_type: String,
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    pub subject: Vec<Subject>,
    pub predicate: serde_json::Value,
}

impl InTotoStatement {
    pub fn new(att: &Attestation) -> Self {
        Self {
            statement_type: "https://in-toto.io/Statement/v1".into(),
            predicate_type: att.predicate_type.uri(),
            subject: att.subject.clone(),
            predicate: att.predicate.clone(),
        }
    }

    /// Inverse of `new`: lift a Statement into the typed `Attestation`.
    pub fn to_attestation(&self) -> Attestation {
        Attestation {
            media_type: "application/vnd.in-toto+json".into(),
            predicate_type: PredicateType::from_uri(&self.predicate_type),
            subject: self.subject.clone(),
            predicate: self.predicate.clone(),
        }
    }
}

/// Build the DSSE pre-authentication-encoding string per spec:
/// `DSSEv1 <type_len> <type> <payload_len> <payload>`.
pub fn dsse_pae(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"DSSEv1 ");
    buf.extend_from_slice(payload_type.len().to_string().as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(payload_type.as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(payload.len().to_string().as_bytes());
    buf.push(b' ');
    buf.extend_from_slice(payload);
    buf
}

/// Sign an attestation, producing a DSSE envelope. `keyid` is opaque (we
/// pass the SHA-256 of the public key for keypair signing; keyless flows
/// substitute the certificate fingerprint).
pub fn sign_attestation(
    att: &Attestation,
    keypair: &Keypair,
    keyid: &str,
) -> Result<DsseEnvelope> {
    let stmt = InTotoStatement::new(att);
    let payload = serde_json::to_vec(&stmt)
        .map_err(|e| SignError::Attestation(format!("statement json: {}", e)))?;
    let pae = dsse_pae(&att.media_type, &payload);
    let sig = keypair.sign(&pae)?;
    Ok(DsseEnvelope {
        payload_type: att.media_type.clone(),
        payload: base64::engine::general_purpose::STANDARD.encode(&payload),
        signatures: vec![DsseSignature {
            keyid: keyid.into(),
            sig: base64::engine::general_purpose::STANDARD.encode(sig),
        }],
    })
}

/// Verify *any* signature in a DSSE envelope against `(algorithm, public_key)`.
pub fn verify_envelope(
    env: &DsseEnvelope,
    algorithm: KeyAlgorithm,
    public_key: &[u8],
) -> Result<Attestation> {
    let payload = base64::engine::general_purpose::STANDARD
        .decode(env.payload.as_bytes())
        .map_err(|e| SignError::Attestation(format!("payload base64: {}", e)))?;
    let pae = dsse_pae(&env.payload_type, &payload);
    let mut last_err: Option<SignError> = None;
    for s in &env.signatures {
        let sig_bytes = match base64::engine::general_purpose::STANDARD.decode(s.sig.as_bytes()) {
            Ok(b) => b,
            Err(e) => {
                last_err = Some(SignError::Attestation(format!("sig base64: {}", e)));
                continue;
            }
        };
        match crate::signature::verify(algorithm, public_key, &pae, &sig_bytes) {
            Ok(()) => {
                let stmt: InTotoStatement = serde_json::from_slice(&payload)
                    .map_err(|e| SignError::Attestation(format!("statement decode: {}", e)))?;
                return Ok(stmt.to_attestation());
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| SignError::Attestation("no signatures".into())))
}

/// Build a SLSA Provenance v1 predicate JSON. Subjects + builder + buildType
/// are the most common audit fields.
pub fn build_slsa_provenance(
    builder_id: &str,
    build_type: &str,
    invocation_uri: &str,
) -> serde_json::Value {
    serde_json::json!({
        "buildDefinition": {
            "buildType": build_type,
            "externalParameters": {"invocationUri": invocation_uri},
            "internalParameters": {},
            "resolvedDependencies": [],
        },
        "runDetails": {
            "builder": {"id": builder_id},
            "metadata": {
                "invocationId": uuid::Uuid::new_v4().to_string(),
                "startedOn": chrono::Utc::now().to_rfc3339(),
            },
            "byproducts": [],
        }
    })
}

/// Build an OpenVEX 0.2.0 predicate JSON.
pub fn build_vex_predicate(
    author: &str,
    statements: Vec<VexStatement>,
) -> serde_json::Value {
    let now = chrono::Utc::now().to_rfc3339();
    serde_json::json!({
        "@context": "https://openvex.dev/ns/v0.2.0",
        "@id": format!("https://cave/vex/{}", uuid::Uuid::new_v4()),
        "author": author,
        "timestamp": now,
        "version": 1,
        "statements": statements,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VexStatement {
    pub vulnerability: String,
    pub products: Vec<String>,
    pub status: VexStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VexStatus {
    NotAffected,
    Affected,
    Fixed,
    UnderInvestigation,
}

/// Helper that constructs a Subject with sha256 digest.
pub fn subject_sha256(name: &str, hex_digest: &str) -> Subject {
    let mut d = BTreeMap::new();
    d.insert("sha256".into(), hex_digest.into());
    Subject {
        name: name.into(),
        digest: d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn slsa_attestation() -> Attestation {
        Attestation {
            media_type: "application/vnd.in-toto+json".into(),
            predicate_type: PredicateType::SlsaProvenance,
            subject: vec![subject_sha256("ghcr.io/cave/x", "abc")],
            predicate: build_slsa_provenance(
                "https://cave/builder",
                "cave-build-v1",
                "https://gh.com/cave/x/run/1",
            ),
        }
    }

    #[test]
    fn dsse_pae_format() {
        let pae = dsse_pae("t", b"hi");
        assert_eq!(pae, b"DSSEv1 1 t 2 hi");
    }

    #[test]
    fn sign_then_verify_envelope() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[10u8; 32]).unwrap();
        let att = slsa_attestation();
        let env = sign_attestation(&att, &kp, "cave-keyid-1").unwrap();
        let back = verify_envelope(&env, KeyAlgorithm::EcdsaP256, kp.public_key_bytes()).unwrap();
        assert_eq!(back.predicate_type, PredicateType::SlsaProvenance);
    }

    #[test]
    fn verify_fails_on_tampered_payload() {
        let kp = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[11u8; 32]).unwrap();
        let att = slsa_attestation();
        let mut env = sign_attestation(&att, &kp, "k").unwrap();
        // Tamper: append a byte.
        let mut decoded = base64::engine::general_purpose::STANDARD
            .decode(env.payload.as_bytes())
            .unwrap();
        decoded.push(b'X');
        env.payload = base64::engine::general_purpose::STANDARD.encode(decoded);
        let err = verify_envelope(&env, KeyAlgorithm::EcdsaP256, kp.public_key_bytes())
            .expect_err("must fail");
        assert!(matches!(err, SignError::Verify(_) | SignError::Attestation(_)));
    }

    #[test]
    fn statement_roundtrip() {
        let att = slsa_attestation();
        let stmt = InTotoStatement::new(&att);
        assert_eq!(stmt.statement_type, "https://in-toto.io/Statement/v1");
        assert_eq!(stmt.predicate_type, "https://slsa.dev/provenance/v1");
        let back = stmt.to_attestation();
        assert_eq!(back.predicate_type, att.predicate_type);
        assert_eq!(back.subject, att.subject);
    }

    #[test]
    fn vex_predicate_builds() {
        let stmts = vec![VexStatement {
            vulnerability: "CVE-2026-0001".into(),
            products: vec!["pkg:oci/cave-runtime@v0.1.0".into()],
            status: VexStatus::NotAffected,
        }];
        let p = build_vex_predicate("alice@example.com", stmts.clone());
        assert_eq!(p["author"], json!("alice@example.com"));
        let s: Vec<VexStatement> = serde_json::from_value(p["statements"].clone()).unwrap();
        assert_eq!(s, stmts);
    }

    #[test]
    fn multiple_signatures_any_passes() {
        let good = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[12u8; 32]).unwrap();
        let bad = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[13u8; 32]).unwrap();
        let att = slsa_attestation();
        let mut env = sign_attestation(&att, &good, "good").unwrap();
        // Prepend a bogus signature; verification should still pass on the good one.
        env.signatures.insert(
            0,
            DsseSignature {
                keyid: "bad".into(),
                sig: base64::engine::general_purpose::STANDARD.encode(bad.sign(b"unrelated").unwrap()),
            },
        );
        verify_envelope(&env, KeyAlgorithm::EcdsaP256, good.public_key_bytes()).unwrap();
    }

    #[test]
    fn no_matching_signature_fails() {
        let signer = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[14u8; 32]).unwrap();
        let other = Keypair::from_seed(KeyAlgorithm::EcdsaP256, &[15u8; 32]).unwrap();
        let att = slsa_attestation();
        let env = sign_attestation(&att, &signer, "k").unwrap();
        let err = verify_envelope(&env, KeyAlgorithm::EcdsaP256, other.public_key_bytes())
            .expect_err("must fail");
        assert!(matches!(err, SignError::Verify(_)));
    }

    #[test]
    fn slsa_provenance_has_builder() {
        let p = build_slsa_provenance("b1", "t1", "u1");
        assert_eq!(p["runDetails"]["builder"]["id"], json!("b1"));
        assert_eq!(p["buildDefinition"]["buildType"], json!("t1"));
    }

    #[test]
    fn subject_helper_attaches_sha256() {
        let s = subject_sha256("name", "deadbeef");
        assert_eq!(s.digest.get("sha256").map(String::as_str), Some("deadbeef"));
    }
}
