// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sign error type — port of pkg/cosign/errors.go.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SignError {
    #[error("invalid digest: {0}")]
    InvalidDigest(String),

    #[error("invalid signature: {0}")]
    InvalidSignature(String),

    #[error("key error: {0}")]
    Key(String),

    #[error("PEM parse error: {0}")]
    Pem(String),

    #[error("certificate error: {0}")]
    Cert(String),

    #[error("bundle parse error: {0}")]
    Bundle(String),

    #[error("attestation error: {0}")]
    Attestation(String),

    #[error("policy violation: {0}")]
    Policy(String),

    #[error("Fulcio error: {0}")]
    Fulcio(String),

    #[error("Rekor error: {0}")]
    Rekor(String),

    #[error("OIDC error: {0}")]
    Oidc(String),

    #[error("transparency log error: {0}")]
    Tlog(String),

    #[error("trusted root error: {0}")]
    TrustedRoot(String),

    #[error("verification failed: {0}")]
    Verify(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("not signed by required identity: {0}")]
    NoMatchingSignature(String),

    #[error("io error: {0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, SignError>;

impl From<std::io::Error> for SignError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<serde_json::Error> for SignError {
    fn from(e: serde_json::Error) -> Self {
        Self::Bundle(format!("json: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_invalid_digest() {
        let e = SignError::InvalidDigest("sha256:".into());
        assert!(e.to_string().contains("invalid digest"));
    }

    #[test]
    fn display_policy() {
        let e = SignError::Policy("issuer mismatch".into());
        assert!(e.to_string().contains("issuer mismatch"));
    }

    #[test]
    fn from_io() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "x");
        let e: SignError = io.into();
        assert!(matches!(e, SignError::Io(_)));
    }

    #[test]
    fn from_serde_json() {
        let r: std::result::Result<serde_json::Value, _> = serde_json::from_str("not json");
        let e: SignError = r.unwrap_err().into();
        assert!(matches!(e, SignError::Bundle(_)));
    }
}
