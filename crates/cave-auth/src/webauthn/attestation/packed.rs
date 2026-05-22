// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// "packed" attestation statement — W3C §8.2.
//
// CBOR shape (self-attestation):
//   { alg: COSE-alg, sig: bytes }
// CBOR shape (basic / x5c):
//   { alg: COSE-alg, sig: bytes, x5c: [DER-cert, ...] }
//
// Verification (port of webauthn4j `PackedAttestationStatementVerifier`):
//
//  - Compute verificationData = authData || clientDataHash
//  - If x5c present (basic attestation):
//      verify sig over verificationData with x5c[0]'s public key
//      (caller is expected to chain-validate x5c separately; cave-auth
//      treats unknown roots as a policy decision — we mark a flag).
//  - If x5c absent (self-attestation):
//      sig MUST be valid under the *credentialPublicKey* from authData.
//      The attestation alg MUST equal the credential alg.

use ciborium::value::Value;

use crate::webauthn::CoseAlg;
use crate::webauthn::WebAuthnError;
use crate::webauthn::cbor;
use crate::webauthn::cose;

#[derive(Debug, Clone)]
pub struct PackedAttStmt {
    pub alg: CoseAlg,
    pub sig: Vec<u8>,
    /// DER-encoded X.509 cert chain — empty for self-attestation.
    pub x5c: Vec<Vec<u8>>,
}

pub fn parse(stmt: &Value) -> Result<PackedAttStmt, WebAuthnError> {
    let alg_v = cbor::map_get_str(stmt, "alg")
        .ok_or_else(|| WebAuthnError::Attestation("packed: missing alg".into()))?;
    let alg = cbor::as_i64(alg_v)?;
    let alg = CoseAlg::from_i64(alg).ok_or(WebAuthnError::UnsupportedAlgorithm(alg))?;
    let sig_v = cbor::map_get_str(stmt, "sig")
        .ok_or_else(|| WebAuthnError::Attestation("packed: missing sig".into()))?;
    let sig = cbor::as_bytes(sig_v)?.to_vec();
    let x5c = match cbor::map_get_str(stmt, "x5c") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|it| cbor::as_bytes(it).map(|b| b.to_vec()))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(WebAuthnError::Attestation("packed: x5c not array".into())),
        None => Vec::new(),
    };
    Ok(PackedAttStmt { alg, sig, x5c })
}

/// Verify a packed *self-attestation* statement.
///
/// `auth_data_raw` is the authenticatorData wire bytes, `client_data_hash` is
/// `SHA-256(clientDataJSON)`. The credential public key must already be
/// parsed and passed in — extracted from authData's attestedCredentialData.
pub fn verify_self(
    stmt: &PackedAttStmt,
    auth_data_raw: &[u8],
    client_data_hash: &[u8],
    credential_key: &cose::CoseKey,
) -> Result<(), WebAuthnError> {
    if !stmt.x5c.is_empty() {
        return Err(WebAuthnError::Attestation(
            "verify_self called with non-empty x5c".into(),
        ));
    }
    if stmt.alg != credential_key.algorithm() {
        return Err(WebAuthnError::Attestation(format!(
            "packed: alg {:?} != credential alg {:?}",
            stmt.alg,
            credential_key.algorithm()
        )));
    }
    let mut data = Vec::with_capacity(auth_data_raw.len() + client_data_hash.len());
    data.extend_from_slice(auth_data_raw);
    data.extend_from_slice(client_data_hash);
    cose::verify(credential_key, &data, &stmt.sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webauthn::cbor;
    use p256::ecdsa::{SigningKey, signature::Signer as _};
    use rand::rngs::OsRng;

    fn cbor_stmt(alg: i64, sig: &[u8], x5c: Option<Vec<Vec<u8>>>) -> Value {
        let mut entries = vec![
            (Value::Text("alg".into()), Value::Integer(alg.into())),
            (Value::Text("sig".into()), Value::Bytes(sig.to_vec())),
        ];
        if let Some(chain) = x5c {
            entries.push((
                Value::Text("x5c".into()),
                Value::Array(chain.into_iter().map(Value::Bytes).collect()),
            ));
        }
        Value::Map(entries)
    }

    #[test]
    fn parse_self_attestation() {
        let v = cbor_stmt(-7, &[1, 2, 3], None);
        let stmt = parse(&v).unwrap();
        assert_eq!(stmt.alg, CoseAlg::Es256);
        assert_eq!(stmt.sig, vec![1, 2, 3]);
        assert!(stmt.x5c.is_empty());
    }

    #[test]
    fn parse_basic_attestation_with_x5c() {
        let v = cbor_stmt(-7, &[1], Some(vec![vec![0xaa], vec![0xbb]]));
        let stmt = parse(&v).unwrap();
        assert_eq!(stmt.x5c.len(), 2);
    }

    #[test]
    fn parse_missing_alg_errors() {
        let v = Value::Map(vec![(Value::Text("sig".into()), Value::Bytes(vec![1]))]);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn parse_unsupported_alg_errors() {
        let v = cbor_stmt(-99, &[1], None);
        assert!(parse(&v).is_err());
    }

    #[test]
    fn verify_self_succeeds_for_correctly_signed_input() {
        // Build a real ES256 keypair, simulate signing over authData||hash.
        let sk = SigningKey::random(&mut OsRng);
        let vk = sk.verifying_key();
        let pt = vk.to_encoded_point(false);
        let x: [u8; 32] = (*pt.x().unwrap()).into();
        let y: [u8; 32] = (*pt.y().unwrap()).into();
        let key = cose::CoseKey::Es256 { x, y };

        let auth_data = vec![0u8; 50];
        let client_data_hash = [0xcd; 32];
        let mut signed_over = auth_data.clone();
        signed_over.extend_from_slice(&client_data_hash);

        let sig: p256::ecdsa::Signature = sk.sign(&signed_over);
        let der = sig.to_der();
        let stmt = PackedAttStmt {
            alg: CoseAlg::Es256,
            sig: der.as_bytes().to_vec(),
            x5c: vec![],
        };
        verify_self(&stmt, &auth_data, &client_data_hash, &key).unwrap();
    }

    #[test]
    fn verify_self_rejects_mismatched_alg() {
        let sk = SigningKey::random(&mut OsRng);
        let vk = sk.verifying_key();
        let pt = vk.to_encoded_point(false);
        let x: [u8; 32] = (*pt.x().unwrap()).into();
        let y: [u8; 32] = (*pt.y().unwrap()).into();
        let key = cose::CoseKey::Es256 { x, y };
        let stmt = PackedAttStmt {
            alg: CoseAlg::EdDsa,
            sig: vec![],
            x5c: vec![],
        };
        assert!(verify_self(&stmt, &[], &[], &key).is_err());
    }

    #[test]
    fn verify_self_with_x5c_is_caller_error() {
        let key = cose::CoseKey::Es256 {
            x: [0; 32],
            y: [0; 32],
        };
        let stmt = PackedAttStmt {
            alg: CoseAlg::Es256,
            sig: vec![],
            x5c: vec![vec![1]],
        };
        assert!(verify_self(&stmt, &[], &[], &key).is_err());
    }

    #[test]
    fn cbor_helpers_roundtrip() {
        let v = cbor_stmt(-7, &[9, 9, 9], None);
        let bytes = cbor::encode(&v).unwrap();
        let parsed_value = cbor::decode(&bytes).unwrap();
        assert_eq!(parsed_value, v);
    }
}
