// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/model/

//! W3C VC Data Model 2.0 — RED phase: skeleton + failing tests.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const VC_CONTEXT_V2: &str = "RED-stub";
pub const TYPE_VERIFIABLE_CREDENTIAL: &str = "RED-stub";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifiableCredential {
    #[serde(rename = "@context", default)]
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub credential_type: Vec<String>,
    #[serde(default = "default_issuer")]
    pub issuer: Issuer,
    #[serde(rename = "validFrom", default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(rename = "validUntil", default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(rename = "credentialSubject", default)]
    pub credential_subject: CredentialSubject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Proof>,
}

fn default_issuer() -> Issuer { Issuer::Uri(String::new()) }

impl VerifiableCredential {
    pub fn new(_issuer_did: impl Into<String>, _subject: CredentialSubject) -> Self {
        Self {
            context: vec![],
            id: None,
            credential_type: vec![],
            issuer: Issuer::Uri(String::new()),
            valid_from: None,
            valid_until: None,
            credential_subject: CredentialSubject::default(),
            proof: None,
        }
    }
    pub fn with_type(self, _t: impl Into<String>) -> Self { self }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Issuer {
    Uri(String),
    Object {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

impl Issuer {
    pub fn id(&self) -> &str { "" }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CredentialSubject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten, default)]
    pub claims: serde_json::Map<String, Value>,
}

impl CredentialSubject {
    pub fn new() -> Self { Self::default() }
    pub fn with_id(self, _id: impl Into<String>) -> Self { self }
    pub fn with_claim(self, _k: impl Into<String>, _v: Value) -> Self { self }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proof {
    #[serde(rename = "type")]
    pub proof_type: String,
    #[serde(rename = "cryptosuite")]
    pub cryptosuite: String,
    pub created: String,
    #[serde(rename = "proofPurpose")]
    pub proof_purpose: String,
    #[serde(rename = "verificationMethod")]
    pub verification_method: String,
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
        assert_eq!(j["@context"][0], "https://www.w3.org/ns/credentials/v2");
    }

    #[test]
    fn vc_type_always_includes_verifiablecredential() {
        let vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        let j: Value = serde_json::to_value(&vc).unwrap();
        assert_eq!(j["type"][0], "VerifiableCredential");
    }

    #[test]
    fn vc_with_type_appends() {
        let vc = VerifiableCredential::new("did:example:123", CredentialSubject::new())
            .with_type("EmployeeCredential");
        assert_eq!(vc.credential_type, vec!["VerifiableCredential", "EmployeeCredential"]);
    }

    #[test]
    fn vc_uses_validfrom_not_issuancedate() {
        let mut vc = VerifiableCredential::new("did:example:123", CredentialSubject::new());
        vc.valid_from = Some("2024-01-01T00:00:00Z".into());
        let j = serde_json::to_value(&vc).unwrap();
        assert!(j.get("validFrom").is_some());
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
        vc.issuer = Issuer::Object { id: "did:example:123".into(), name: Some("Cave Inc".into()) };
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
        let mut vc = VerifiableCredential::new("did:example:issuer", cs).with_type("EmployeeCredential");
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
