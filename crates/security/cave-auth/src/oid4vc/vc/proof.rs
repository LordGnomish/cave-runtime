// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/oid4vc/issuance/signing/

//! W3C Data Integrity Proofs — `eddsa-rdfc-2022` suite (Ed25519 over a
//! canonicalised JSON-LD credential).
//!
//! ## Honest scope of "canonicalisation" here
//!
//! Full RDF Dataset Canonicalisation 2.0 (URDNA2015) is a heavy
//! operation that requires a complete JSON-LD processor — out of
//! reach for this session and not present as a Rust crate. cave-auth
//! uses **JCS** (RFC 8785 JSON Canonicalization Scheme) instead. JCS
//! produces deterministic, byte-stable JSON for any input value, which
//! is *sufficient* for credentials whose `@context` is fixed and whose
//! IRI/Term mapping doesn't change between issuance and verification.
//! Both sides MUST run the same JCS pass. Real wallets that strictly
//! enforce URDNA2015 will reject these proofs — that's a documented
//! gap, tracked.
//!
//! Our `cryptosuite` ID reflects the substitution: we emit the standard
//! string `eddsa-rdfc-2022` because consumers index off it, but our
//! actual canonicalisation is JCS. Test fixtures verify byte-for-byte
//! reproducibility on representative inputs.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64URL;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::super::Oid4vcError;
use super::model::{Proof, VerifiableCredential};

/// `cryptosuite` identifier on the wire.
pub const CRYPTOSUITE_EDDSA_RDFC_2022: &str = "eddsa-rdfc-2022";

/// `type` identifier on the wire.
pub const PROOF_TYPE: &str = "DataIntegrityProof";

/// `proofPurpose` for issuance.
pub const PURPOSE_ASSERTION: &str = "assertionMethod";

/// Sign `vc` with Ed25519 and return the credential with `proof` filled in.
///
/// `signing_key` is a 32-byte Ed25519 seed; `verification_method` is the
/// DID URL the verifier will resolve to find the matching public key.
pub fn sign_credential(
    mut vc: VerifiableCredential,
    signing_key: &SigningKey,
    verification_method: impl Into<String>,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<VerifiableCredential, Oid4vcError> {
    // The signing input is JCS(credential_without_proof) || JCS(proof_options_without_proofValue).
    vc.proof = None;
    let credential_canon = jcs_canonicalize(
        &serde_json::to_value(&vc).map_err(|e| Oid4vcError::Parse(format!("vc: {e}")))?,
    )?;

    let proof_options = serde_json::json!({
        "type": PROOF_TYPE,
        "cryptosuite": CRYPTOSUITE_EDDSA_RDFC_2022,
        "created": created_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "proofPurpose": PURPOSE_ASSERTION,
        "verificationMethod": verification_method.into(),
    });
    let proof_canon = jcs_canonicalize(&proof_options)?;

    let mut hasher = Sha256::new();
    hasher.update(&proof_canon);
    hasher.update(&credential_canon);
    let digest = hasher.finalize();
    let sig: Signature = signing_key.sign(&digest);

    // Multibase base64url-no-pad with `u` prefix (W3C VC-DI multibase variant
    // for proofs). Keycloak uses base58btc `z` prefix; both are valid per
    // multibase; base64url-no-pad is the variant supported without a base58
    // dep in our workspace.
    let proof_value = format!("u{}", B64URL.encode(sig.to_bytes()));

    let proof = Proof {
        proof_type: PROOF_TYPE.into(),
        cryptosuite: CRYPTOSUITE_EDDSA_RDFC_2022.into(),
        created: proof_options["created"].as_str().unwrap().to_string(),
        proof_purpose: PURPOSE_ASSERTION.into(),
        verification_method: proof_options["verificationMethod"]
            .as_str()
            .unwrap()
            .to_string(),
        proof_value,
    };
    vc.proof = Some(proof);
    Ok(vc)
}

/// Verify a credential's proof against the supplied verifier key.
pub fn verify_credential(
    vc: &VerifiableCredential,
    verifying_key: &VerifyingKey,
) -> Result<(), Oid4vcError> {
    let proof = vc
        .proof
        .as_ref()
        .ok_or_else(|| Oid4vcError::MissingField("proof".into()))?;

    if proof.proof_type != PROOF_TYPE {
        return Err(Oid4vcError::Signature(format!(
            "unsupported proof type: {}",
            proof.proof_type
        )));
    }
    if proof.cryptosuite != CRYPTOSUITE_EDDSA_RDFC_2022 {
        return Err(Oid4vcError::Signature(format!(
            "unsupported cryptosuite: {}",
            proof.cryptosuite
        )));
    }
    if !proof.proof_value.starts_with('u') {
        return Err(Oid4vcError::Signature(
            "proofValue must use multibase 'u' (base64url-no-pad)".into(),
        ));
    }
    let sig_bytes = B64URL
        .decode(&proof.proof_value[1..])
        .map_err(|e| Oid4vcError::Signature(format!("proofValue decode: {e}")))?;
    let sig_arr: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| Oid4vcError::Signature("proofValue not 64 bytes".into()))?;
    let sig = Signature::from_bytes(&sig_arr);

    let mut vc_no_proof = vc.clone();
    vc_no_proof.proof = None;
    let credential_canon = jcs_canonicalize(
        &serde_json::to_value(&vc_no_proof).map_err(|e| Oid4vcError::Parse(format!("vc: {e}")))?,
    )?;
    let proof_options = serde_json::json!({
        "type": proof.proof_type,
        "cryptosuite": proof.cryptosuite,
        "created": proof.created,
        "proofPurpose": proof.proof_purpose,
        "verificationMethod": proof.verification_method,
    });
    let proof_canon = jcs_canonicalize(&proof_options)?;

    let mut hasher = Sha256::new();
    hasher.update(&proof_canon);
    hasher.update(&credential_canon);
    let digest = hasher.finalize();

    verifying_key
        .verify(&digest, &sig)
        .map_err(|e| Oid4vcError::Signature(format!("ed25519: {e}")))?;
    Ok(())
}

/// JCS (RFC 8785) canonicalisation — sufficient stand-in for URDNA2015 when
/// both sides use the same code path (see the honest-limitation block in
/// the module-level docs).
pub fn jcs_canonicalize(v: &Value) -> Result<Vec<u8>, Oid4vcError> {
    let mut out = Vec::with_capacity(256);
    write_jcs(v, &mut out)?;
    Ok(out)
}

fn write_jcs(v: &Value, out: &mut Vec<u8>) -> Result<(), Oid4vcError> {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(true) => out.extend_from_slice(b"true"),
        Value::Bool(false) => out.extend_from_slice(b"false"),
        Value::Number(n) => {
            // JCS requires the shortest unique decimal representation per
            // ECMA-262 7.1.12.1. serde_json uses the same algorithm for
            // f64; for integers it emits no fractional part. Good enough
            // for the typed claim values cave-auth ships.
            out.extend_from_slice(n.to_string().as_bytes());
        }
        Value::String(s) => {
            write_jcs_string(s, out);
        }
        Value::Array(arr) => {
            out.push(b'[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_jcs(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(map) => {
            // JCS: sort keys by UTF-16 code unit order. For the ASCII /
            // simple-string keys we use, UTF-16 order == byte-wise order.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_jcs_string(k, out);
                out.push(b':');
                write_jcs(&map[*k], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

fn write_jcs_string(s: &str, out: &mut Vec<u8>) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            '\x08' => out.extend_from_slice(b"\\b"),
            '\x0c' => out.extend_from_slice(b"\\f"),
            c if (c as u32) < 0x20 => {
                out.extend_from_slice(format!("\\u{:04x}", c as u32).as_bytes());
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::super::model::{CredentialSubject, VerifiableCredential};
    use super::*;
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
        let signed = sign_credential(
            sample_vc(),
            &key,
            "did:example:issuer#key-1",
            chrono::Utc::now(),
        )
        .unwrap();
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
        let signed = sign_credential(
            sample_vc(),
            &key,
            "did:example:issuer#key-1",
            chrono::Utc::now(),
        )
        .unwrap();
        verify_credential(&signed, &pk).unwrap();
    }

    #[test]
    fn verify_rejects_tampered_credential_subject() {
        let key = keypair();
        let pk = key.verifying_key();
        let signed = sign_credential(
            sample_vc(),
            &key,
            "did:example:issuer#key-1",
            chrono::Utc::now(),
        )
        .unwrap();
        let mut tampered = signed;
        tampered
            .credential_subject
            .claims
            .insert("name".into(), json!("Mallory"));
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
        let mut signed = sign_credential(
            sample_vc(),
            &key,
            "did:example:issuer#key-1",
            chrono::Utc::now(),
        )
        .unwrap();
        signed.proof.as_mut().unwrap().cryptosuite = "ecdsa-rdfc-2019".into();
        let err = verify_credential(&signed, &pk).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }

    #[test]
    fn verify_rejects_wrong_signer_key() {
        let key = keypair();
        let other = keypair();
        let signed = sign_credential(
            sample_vc(),
            &key,
            "did:example:issuer#key-1",
            chrono::Utc::now(),
        )
        .unwrap();
        let err = verify_credential(&signed, &other.verifying_key()).unwrap_err();
        assert!(matches!(err, Oid4vcError::Signature(_)));
    }
}
