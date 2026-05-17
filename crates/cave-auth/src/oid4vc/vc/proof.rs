// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/signing/

//! Data Integrity Proofs — RED phase.

use ed25519_dalek::{SigningKey, VerifyingKey};
use serde_json::Value;

use super::model::VerifiableCredential;
use super::super::Oid4vcError;

pub const CRYPTOSUITE_EDDSA_RDFC_2022: &str = "eddsa-rdfc-2022";
pub const PROOF_TYPE: &str = "DataIntegrityProof";
pub const PURPOSE_ASSERTION: &str = "assertionMethod";

pub fn sign_credential(
    _vc: VerifiableCredential,
    _signing_key: &SigningKey,
    _verification_method: impl Into<String>,
    _created_at: chrono::DateTime<chrono::Utc>,
) -> Result<VerifiableCredential, Oid4vcError> {
    Err(Oid4vcError::Parse("RED-phase stub".into()))
}

pub fn verify_credential(_vc: &VerifiableCredential, _vk: &VerifyingKey) -> Result<(), Oid4vcError> {
    Err(Oid4vcError::Parse("RED-phase stub".into()))
}

pub fn jcs_canonicalize(_v: &Value) -> Result<Vec<u8>, Oid4vcError> {
    Err(Oid4vcError::Parse("RED-phase stub".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::model::{CredentialSubject, VerifiableCredential};
    use rand::RngCore;
    use rand::rngs::OsRng;
    use serde_json::json;

    fn keypair() -> SigningKey {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        SigningKey::from_bytes(&seed)
    }

    fn sample_vc() -> VerifiableCredential {
        let cs = CredentialSubject::new()
            .with_id("did:example:alice")
            .with_claim("name", json!("Alice"))
            .with_claim("age", json!(30));
        VerifiableCredential::new("did:example:issuer", cs).with_type("EmployeeCredential")
    }

    #[test]
    fn jcs_object_keys_sort_alphabetically() {
        let v = json!({"b": 1, "a": 2, "c": 3});
        let canon = jcs_canonicalize(&v).unwrap();
        assert_eq!(canon, br#"{"a":2,"b":1,"c":3}"#);
    }

    #[test]
    fn jcs_strings_escape_quotes_and_backslashes() {
        let v = json!("he said \"hi\" and \\");
        let canon = jcs_canonicalize(&v).unwrap();
        assert_eq!(canon, br#""he said \"hi\" and \\""#);
    }

    #[test]
    fn jcs_handles_control_chars() {
        let v = json!("line1\nline2\ttab");
        let canon = jcs_canonicalize(&v).unwrap();
        assert_eq!(canon, br#""line1\nline2\ttab""#);
    }

    #[test]
    fn jcs_array_preserves_order() {
        let v = json!([3, 1, 2]);
        let canon = jcs_canonicalize(&v).unwrap();
        assert_eq!(canon, b"[3,1,2]");
    }

    #[test]
    fn jcs_canonicalisation_is_deterministic() {
        let v = json!({"x": [1, 2, {"b": 1, "a": 2}], "y": null});
        let a = jcs_canonicalize(&v).unwrap();
        let b = jcs_canonicalize(&v).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn jcs_handles_nested_objects() {
        let v = json!({"outer": {"z": 1, "a": 2}});
        let canon = jcs_canonicalize(&v).unwrap();
        assert_eq!(canon, br#"{"outer":{"a":2,"z":1}}"#);
    }

    #[test]
    fn sign_attaches_proof_to_credential() {
        let key = keypair();
        let signed = sign_credential(sample_vc(), &key, "did:example:issuer#key-1", chrono::Utc::now()).unwrap();
        assert!(signed.proof.is_some());
        let p = signed.proof.as_ref().unwrap();
        assert_eq!(p.proof_type, "DataIntegrityProof");
        assert_eq!(p.cryptosuite, "eddsa-rdfc-2022");
        assert_eq!(p.proof_purpose, "assertionMethod");
        assert_eq!(p.verification_method, "did:example:issuer#key-1");
        assert!(p.proof_value.starts_with('u'));
    }

    #[test]
    fn sign_then_verify_round_trips() {
        let key = keypair();
        let pk = key.verifying_key();
        let signed = sign_credential(sample_vc(), &key, "did:example:issuer#key-1", chrono::Utc::now()).unwrap();
        verify_credential(&signed, &pk).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_credential_subject() {
        let key = keypair();
        let pk = key.verifying_key();
        let signed = sign_credential(sample_vc(), &key, "did:example:issuer#key-1", chrono::Utc::now()).unwrap();
        let mut tampered = signed.clone();
        tampered.credential_subject.claims.insert("name".into(), json!("Mallory"));
        let err = verify_credential(&tampered, &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }

    #[test]
    fn verify_rejects_missing_proof() {
        let pk = keypair().verifying_key();
        let err = verify_credential(&sample_vc(), &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::MissingField(_)));
    }

    #[test]
    fn verify_rejects_wrong_cryptosuite() {
        let key = keypair();
        let pk = key.verifying_key();
        let mut signed = sign_credential(sample_vc(), &key, "did:example:issuer#key-1", chrono::Utc::now()).unwrap();
        signed.proof.as_mut().unwrap().cryptosuite = "ecdsa-rdfc-2019".into();
        let err = verify_credential(&signed, &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }

    #[test]
    fn verify_rejects_wrong_signer_key() {
        let key = keypair();
        let other = keypair();
        let signed = sign_credential(sample_vc(), &key, "did:example:issuer#key-1", chrono::Utc::now()).unwrap();
        let err = verify_credential(&signed, &other.verifying_key()).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }
}
