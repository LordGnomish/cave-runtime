// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// WebAuthn / FIDO2 / passkey ceremony engine for cave-auth.
//
// Module map (line-by-line port of webauthn4j v0.24.0 + Keycloak v22 webauthn):
//
// | Submodule | Upstream Java | Responsibility |
// |-----------|---------------|----------------|
// | [`cbor`] | webauthn4j `data.attestation.AttestationObjectConverter` | Minimal CBOR decoder for attestation objects |
// | [`cose`] | webauthn4j `data.attestation.authenticator.CredentialPublicKey` | COSE_Key parse (ES256 / RS256 / EdDSA) |
// | [`client_data`] | webauthn4j `data.client.CollectedClientData` | clientDataJSON parsing + challenge / origin check |
// | [`authenticator_data`] | webauthn4j `data.attestation.authenticator.AuthenticatorData` | Flags, signCount, AAGUID, attested credential data, extensions |
// | [`attestation`] | webauthn4j `verifier.attestation.statement.*` | packed / tpm / android-key / none statement parsers |
// | [`registration`] | webauthn4j `WebAuthnRegistrationManager` | Registration ceremony |
// | [`authentication`] | webauthn4j `WebAuthnAuthenticationManager` | Assertion ceremony |
// | [`credential_store`] | Keycloak `WebAuthnCredentialProvider` | Credential persistence trait + in-memory backend |
// | [`resident_key`] | webauthn4j passkey extensions | Discoverable credential (passkey) flow |
//
// Crypto deps: `ciborium`, `coset`, `p256`, `rsa`, `ed25519-dalek`, `sha2`.

pub mod attestation;
pub mod authentication;
pub mod authenticator_data;
pub mod cbor;
pub mod client_data;
pub mod cose;
pub mod credential_store;
pub mod registration;
pub mod resident_key;

#[cfg(test)]
mod tests {
    pub mod upstream_port;
}

use thiserror::Error;

/// Top-level error returned by every WebAuthn ceremony entry point.
///
/// Port of `webauthn4j` `ValidationException` hierarchy collapsed into a
/// single tagged enum.
#[derive(Debug, Error)]
pub enum WebAuthnError {
    #[error("malformed CBOR: {0}")]
    Cbor(String),
    #[error("malformed COSE_Key: {0}")]
    Cose(String),
    #[error("clientDataJSON: {0}")]
    ClientData(String),
    #[error("authenticatorData: {0}")]
    AuthenticatorData(String),
    #[error("attestation: {0}")]
    Attestation(String),
    #[error("registration: {0}")]
    Registration(String),
    #[error("authentication: {0}")]
    Authentication(String),
    #[error("credential not found: {0}")]
    CredentialNotFound(String),
    #[error("signature: {0}")]
    Signature(String),
    #[error("unsupported algorithm: {0}")]
    UnsupportedAlgorithm(i64),
}

/// COSE algorithm identifiers (RFC 8152 §16.4).
///
/// Mirrors webauthn4j `COSEAlgorithmIdentifier`. We only port the algorithms
/// allowed by the W3C WebAuthn L2 algorithm registry — `ES256`, `RS256`,
/// `EdDSA`. Newer FIDO2 ECDAA / ML-DSA hybrids are deferred (see
/// parity.manifest.toml).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i64)]
pub enum CoseAlg {
    /// ECDSA w/ SHA-256 over P-256.
    Es256 = -7,
    /// EdDSA over Curve25519.
    EdDsa = -8,
    /// RSASSA-PKCS1-v1_5 w/ SHA-256.
    Rs256 = -257,
}

impl CoseAlg {
    pub fn from_i64(v: i64) -> Option<Self> {
        match v {
            -7 => Some(Self::Es256),
            -8 => Some(Self::EdDsa),
            -257 => Some(Self::Rs256),
            _ => None,
        }
    }
}
