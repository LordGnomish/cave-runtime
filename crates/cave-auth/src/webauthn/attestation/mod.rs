// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j/webauthn4j@82345b8
//   webauthn4j-core/src/main/java/com/webauthn4j/validator/attestation/statement/
//
// Attestation format dispatch.
//
// The W3C spec defines seven IANA-registered formats; we port a real
// verifier for each only where the cryptography fits inside a single
// 4-track sprint:
//
//   fmt              Status in this port
//   ───────────────  ────────────────────────────────────────────────
//   none             Real — no statement to verify
//   packed (self)    Real — credential key signs sig(authData||hash)
//   packed (x5c)     Honest scope-cut: chain validation is heavy
//   fido-u2f         Honest scope-cut: legacy U2F binding
//   apple            Honest scope-cut: Apple AppAttest nonce chain
//   android-key      Honest scope-cut: ASN.1 KeyDescription parsing
//   android-safetynet  Honest scope-cut: SafetyNet attestation JWT
//   tpm              Honest scope-cut: TPM 2.0 quote + EK chain
//
// Unsupported formats return `AttestationError::Unsupported` so the
// registration ceremony rejects them — never a silent pass.

use crate::webauthn::model::AttestedCredentialData;
use crate::webauthn::registration::AttestationTrustPath;

pub mod android_key;
pub mod android_safetynet;
pub mod apple;
pub mod fido_u2f;
pub mod none;
pub mod packed;
pub mod tpm;

/// Decoded attestation statement payload + transport hash inputs.
#[derive(Debug, Clone)]
pub struct AttestationStatement {
    pub fmt: String,
    pub att_stmt: ciborium::Value,
    pub auth_data_bytes: Vec<u8>,
    pub client_data_hash: [u8; 32],
    pub attested: AttestedCredentialData,
}

#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("attestation format {0:?} is not yet enforced in this build")]
    Unsupported(String),
    #[error("attStmt is not a CBOR map")]
    BadStatement,
    #[error("attStmt missing required field {0:?}")]
    MissingField(&'static str),
    #[error("attStmt {field} has wrong CBOR type")]
    WrongType { field: &'static str },
    #[error("unsupported COSE algorithm {0}")]
    UnsupportedAlg(i64),
    #[error("algorithm in attStmt does not match credential public key")]
    AlgMismatch,
    #[error("signature verification failed")]
    BadSignature,
    #[error("public key reconstruction failed: {0}")]
    BadKey(String),
}

/// Dispatch to the per-format verifier.
pub fn verify(stmt: &AttestationStatement) -> Result<AttestationTrustPath, AttestationError> {
    match stmt.fmt.as_str() {
        "none" => none::verify(stmt),
        "packed" => packed::verify(stmt),
        "fido-u2f" => fido_u2f::verify(stmt),
        "apple" => apple::verify(stmt),
        "android-key" => android_key::verify(stmt),
        "android-safetynet" => android_safetynet::verify(stmt),
        "tpm" => tpm::verify(stmt),
        other => Err(AttestationError::Unsupported(other.into())),
    }
}
