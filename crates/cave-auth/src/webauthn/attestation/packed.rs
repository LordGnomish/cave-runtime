// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/packed/PackedAttestationStatementValidator.java
//
// "packed" attestation — W3C §8.2.  attStmt = { alg, sig, [x5c] }.
//
//   alg = COSE algorithm identifier
//   sig = signature over (authenticatorData || clientDataHash)
//   x5c = optional X.509 cert chain (Basic / AttCA path)
//
// Two paths:
//   1. SELF — no x5c.  Signature is verified with the credential's own
//      public key (the COSE_Key we just parsed out of authData).  We
//      DO implement this — the cryptography fits inside ES256/ES384/
//      RS256/EdDSA primitives already present in the workspace.
//   2. x5c — full cert chain validation including AAGUID matching, MDS
//      lookup, and CRL.  Honest scope-cut for OSS launch.

use super::{AttestationError, AttestationStatement};
use crate::webauthn::cose::{CoseAlgorithm, CoseKey};
use crate::webauthn::registration::AttestationTrustPath;

pub fn verify(stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    let map = match &stmt.att_stmt {
        ciborium::Value::Map(m) => m,
        _ => return Err(AttestationError::BadStatement),
    };
    let mut alg: Option<i64> = None;
    let mut sig: Option<Vec<u8>> = None;
    let mut has_x5c = false;
    for (k, v) in map.iter() {
        match k {
            ciborium::Value::Text(t) if t == "alg" => {
                if let ciborium::Value::Integer(i) = v {
                    alg = i128::from(*i).try_into().ok();
                }
            }
            ciborium::Value::Text(t) if t == "sig" => {
                if let ciborium::Value::Bytes(b) = v {
                    sig = Some(b.clone());
                }
            }
            ciborium::Value::Text(t) if t == "x5c" => {
                has_x5c = true;
            }
            _ => {}
        }
    }
    let alg_raw = alg.ok_or(AttestationError::MissingField("alg"))?;
    let sig = sig.ok_or(AttestationError::MissingField("sig"))?;
    let alg = CoseAlgorithm::from_i64(alg_raw).ok_or(AttestationError::UnsupportedAlg(alg_raw))?;

    if has_x5c {
        // Real x5c chain validation is deferred until we have ASN.1 + a
        // trusted MDS root store.  Refuse explicitly so the caller knows.
        return Err(AttestationError::Unsupported(
            "packed/x5c — full chain validation not enabled in this build".into(),
        ));
    }

    // SELF path: signature must match credential public key's algorithm.
    let cred_key = &stmt.attested.public_key;
    if cred_key.alg() != alg {
        return Err(AttestationError::AlgMismatch);
    }

    let mut signed = Vec::with_capacity(stmt.auth_data_bytes.len() + 32);
    signed.extend_from_slice(&stmt.auth_data_bytes);
    signed.extend_from_slice(&stmt.client_data_hash);

    verify_signature(cred_key, &signed, &sig)?;
    Ok(AttestationTrustPath::SelfAttested)
}

/// Verify `signature` over `data` using the supplied COSE key.
pub(crate) fn verify_signature(
    key: &CoseKey,
    data: &[u8],
    sig: &[u8],
) -> Result<(), AttestationError> {
    match key {
        CoseKey::Ec2 {
            alg: CoseAlgorithm::Es256,
            x,
            y,
            ..
        } => verify_es256(x, y, data, sig),
        CoseKey::Ec2 {
            alg: CoseAlgorithm::Es384,
            x,
            y,
            ..
        } => verify_es384(x, y, data, sig),
        CoseKey::Rsa { n, e, .. } => verify_rs256(n, e, data, sig),
        CoseKey::Okp {
            alg: CoseAlgorithm::EdDsa,
            x,
            ..
        } => verify_ed25519(x, data, sig),
        _ => Err(AttestationError::UnsupportedAlg(key.alg().as_i64())),
    }
}

fn verify_es256(x: &[u8], y: &[u8], data: &[u8], sig: &[u8]) -> Result<(), AttestationError> {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p256::EncodedPoint;
    let pt = EncodedPoint::from_affine_coordinates(x.into(), y.into(), false);
    let vk = VerifyingKey::from_encoded_point(&pt).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    let signature = Signature::from_der(sig).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    vk.verify(data, &signature)
        .map_err(|_| AttestationError::BadSignature)
}

fn verify_es384(x: &[u8], y: &[u8], data: &[u8], sig: &[u8]) -> Result<(), AttestationError> {
    use p384::ecdsa::{signature::Verifier, Signature, VerifyingKey};
    use p384::EncodedPoint;
    let pt = EncodedPoint::from_affine_coordinates(x.into(), y.into(), false);
    let vk = VerifyingKey::from_encoded_point(&pt).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    let signature = Signature::from_der(sig).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    vk.verify(data, &signature)
        .map_err(|_| AttestationError::BadSignature)
}

fn verify_rs256(n: &[u8], e: &[u8], data: &[u8], sig: &[u8]) -> Result<(), AttestationError> {
    use rsa::pkcs1v15::{Signature as RsaSig, VerifyingKey as RsaVK};
    use rsa::signature::Verifier;
    let n = rsa::BigUint::from_bytes_be(n);
    let e = rsa::BigUint::from_bytes_be(e);
    let pk = rsa::RsaPublicKey::new(n, e).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    let vk: RsaVK<sha2::Sha256> = RsaVK::new(pk);
    let signature = RsaSig::try_from(sig).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    vk.verify(data, &signature)
        .map_err(|_| AttestationError::BadSignature)
}

fn verify_ed25519(x: &[u8], data: &[u8], sig: &[u8]) -> Result<(), AttestationError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let key_bytes: &[u8; 32] = x
        .try_into()
        .map_err(|_| AttestationError::BadKey("Ed25519 key must be 32 bytes".into()))?;
    let vk = VerifyingKey::from_bytes(key_bytes).map_err(|e| AttestationError::BadKey(e.to_string()))?;
    let sig_bytes: &[u8; 64] = sig
        .try_into()
        .map_err(|_| AttestationError::BadSignature)?;
    let signature = Signature::from_bytes(sig_bytes);
    vk.verify(data, &signature)
        .map_err(|_| AttestationError::BadSignature)
}
