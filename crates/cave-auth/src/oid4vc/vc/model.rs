// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/model/

//! W3C Verifiable Credentials Data Model 2.0 — minimal JSON-LD shape
//! enough to issue and verify a simple identity credential.
//!
//! Per the W3C spec a VC carries:
//! * `@context` array (always starts with `https://www.w3.org/ns/credentials/v2`)
//! * `type` array (always includes `"VerifiableCredential"`)
//! * `issuer` (URI or object with `id`)
//! * `credentialSubject` (single object or array)
//! * optional `validFrom` / `validUntil` (2.0 renamed from 1.1's
//!   `issuanceDate` / `expirationDate`)
//! * `proof` — DataIntegrityProof (`eddsa-rdfc-2022` or `ecdsa-rdfc-2019`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default context URL list for VC 2.0 credentials.
pub const VC_CONTEXT_V2: &str = "https://www.w3.org/ns/credentials/v2";

/// Default VC type.
pub const TYPE_VERIFIABLE_CREDENTIAL: &str = "VerifiableCredential";

/// W3C VC Data Model 2.0 credential.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifiableCredential {
    /// JSON-LD `@context` — must include VC 2.0 context as first element.
    #[serde(rename = "@context")]
    pub context: Vec<String>,
    /// `id` URI uniquely identifying this credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `type` array — must include `VerifiableCredential`.
    #[serde(rename = "type")]
    pub credential_type: Vec<String>,
    /// `issuer` — URI string, or object with `id` + extra metadata.
    pub issuer: Issuer,
    /// `validFrom` (RFC3339 timestamp). Replaces 1.1's `issuanceDate`.
    #[serde(rename = "validFrom", skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    /// `validUntil` (RFC3339 timestamp). Replaces 1.1's `expirationDate`.
    #[serde(rename = "validUntil", skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    /// `credentialSubject` — single subject object.
    #[serde(rename = "credentialSubject")]
    pub credential_subject: CredentialSubject,
    /// `proof` — optional, present after [`crate::oid4vc::vc::proof::sign_credential`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof: Option<Proof>,
}

impl VerifiableCredential {
    /// Build a new VC 2.0 credential with default contexts/types.
    pub fn new(issuer_did: impl Into<String>, subject: CredentialSubject) -> Self {
        Self {
            context: vec![VC_CONTEXT_V2.to_string()],
            id: None,
            credential_type: vec![TYPE_VERIFIABLE_CREDENTIAL.to_string()],
            issuer: Issuer::Uri(issuer_did.into()),
            valid_from: None,
            valid_until: None,
            credential_subject: subject,
            proof: None,
        }
    }

    /// Add an extra `type` (e.g. `"EmployeeCredential"`).
    pub fn with_type(mut self, t: impl Into<String>) -> Self {
        self.credential_type.push(t.into());
        self
    }
}

/// Issuer — may be a bare URI or a structured object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Issuer {
    /// Bare URI form.
    Uri(String),
    /// Structured form (`{"id": "did:...", "name": "..."}`).
    Object {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl Issuer {
    /// The DID/URI identifier regardless of shape.
    pub fn id(&self) -> &str {
        match self {
            Issuer::Uri(s) => s,
            Issuer::Object { id, .. } => id,
        }
    }
}

/// Credential subject — open-ended key/value map keyed by claim name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CredentialSubject {
    /// Optional subject `id` URI (DID).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Free-form claims as a JSON object.
    #[serde(flatten)]
    pub claims: serde_json::Map<String, Value>,
}

impl CredentialSubject {
    pub fn new() -> Self {
        Self { id: None, claims: serde_json::Map::new() }
    }
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn with_claim(mut self, key: impl Into<String>, value: Value) -> Self {
        self.claims.insert(key.into(), value);
        self
    }
}

/// W3C VC-DI 1.0 DataIntegrityProof.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proof {
    /// Always `"DataIntegrityProof"` for this module.
    #[serde(rename = "type")]
    pub proof_type: String,
    /// Cryptographic suite — `eddsa-rdfc-2022` here.
    #[serde(rename = "cryptosuite")]
    pub cryptosuite: String,
    /// `created` instant (RFC3339).
    pub created: String,
    /// `proofPurpose` — `assertionMethod` for issuance.
    #[serde(rename = "proofPurpose")]
    pub proof_purpose: String,
    /// `verificationMethod` URI — a DID URL pointing at the issuer's key.
    #[serde(rename = "verificationMethod")]
    pub verification_method: String,
    /// `proofValue` — multibase-encoded signature bytes (base58btc).
    #[serde(rename = "proofValue")]
    pub proof_value: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vc_serialises_with_w3c_context() {
        let vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        let j: Value = serde_json::to_value(&vc).unwrap();
        assert_eq!(j["@context"][0], VC_CONTEXT_V2);
    }

    #[test]
    fn vc_type_always_includes_verifiablecredential() {
        let vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        let j: Value = serde_json::to_value(&vc).unwrap();
        assert_eq!(j["type"][0], TYPE_VERIFIABLE_CREDENTIAL);
    }

    #[test]
    fn vc_with_type_appends() {
        let vc = VerifiableCredential::new("did:example:123", CredentialSubject::new())
            .with_type("EmployeeCredential");
        assert_eq!(
            vc.credential_type,
            vec!["VerifiableCredential", "EmployeeCredential"]
        );
    }

    #[test]
    fn vc_uses_validfrom_not_issuancedate() {
        let mut vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        vc.valid_from = Some("2024-01-01T00:00:00Z".into());
        let j = serde_json::to_value(&vc).unwrap();
        assert!(j.get("validFrom").is_some());
        // VC 2.0 renamed the field; old name must NOT appear.
        assert!(j.get("issuanceDate").is_none());
    }

    #[test]
    fn vc_uses_validuntil_not_expirationdate() {
        let mut vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        vc.valid_until = Some("2025-01-01T00:00:00Z".into());
        let j = serde_json::to_value(&vc).unwrap();
        assert!(j.get("validUntil").is_some());
        assert!(j.get("expirationDate").is_none());
    }

    #[test]
    fn issuer_uri_serialises_as_string() {
        let vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        let j = serde_json::to_value(&vc).unwrap();
        assert_eq!(j["issuer"], "did:example:123");
    }

    #[test]
    fn issuer_object_serialises_as_object() {
        let mut vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        vc.issuer = Issuer::Object {
            id: "did:example:123".into(),
            name: Some("Cave Inc".into()),
        };
        let j = serde_json::to_value(&vc).unwrap();
        assert_eq!(j["issuer"]["id"], "did:example:123");
        assert_eq!(j["issuer"]["name"], "Cave Inc");
    }

    #[test]
    fn issuer_id_works_for_both_shapes() {
        let i1 = Issuer::Uri("did:a".into());
        let i2 = Issuer::Object { id: "did:b".into(), name: None };
        assert_eq!(i1.id(), "did:a");
        assert_eq!(i2.id(), "did:b");
    }

    #[test]
    fn credential_subject_flattens_claims_to_root() {
        let cs = CredentialSubject::new()
            .with_id("did:example:alice")
            .with_claim("name", json!("Alice"))
            .with_claim("age", json!(30));
        let j = serde_json::to_value(&cs).unwrap();
        assert_eq!(j["id"], "did:example:alice");
        assert_eq!(j["name"], "Alice");
        assert_eq!(j["age"], 30);
    }

    #[test]
    fn vc_roundtrip_through_json() {
        let cs = CredentialSubject::new()
            .with_id("did:example:alice")
            .with_claim("name", json!("Alice"));
        let mut vc = VerifiableCredential::new("did:example:issuer", cs)
            .with_type("EmployeeCredential");
        vc.valid_from = Some("2024-01-01T00:00:00Z".into());
        let j = serde_json::to_string(&vc).unwrap();
        let back: VerifiableCredential = serde_json::from_str(&j).unwrap();
        assert_eq!(back, vc);
    }

    #[test]
    fn proof_serialises_with_correct_field_names() {
        let p = Proof {
            proof_type: "DataIntegrityProof".into(),
            cryptosuite: "eddsa-rdfc-2022".into(),
            created: "2024-01-01T00:00:00Z".into(),
            proof_purpose: "assertionMethod".into(),
            verification_method: "did:example:1#key-1".into(),
            proof_value: "zABC".into(),
        };
        let j = serde_json::to_value(&p).unwrap();
        assert_eq!(j["type"], "DataIntegrityProof");
        assert_eq!(j["cryptosuite"], "eddsa-rdfc-2022");
        assert_eq!(j["proofPurpose"], "assertionMethod");
        assert_eq!(j["verificationMethod"], "did:example:1#key-1");
        assert_eq!(j["proofValue"], "zABC");
    }
}
