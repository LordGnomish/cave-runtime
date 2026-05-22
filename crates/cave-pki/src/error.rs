// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use thiserror::Error;

pub type PkiResult<T> = Result<T, PkiError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PkiError {
    /// Cite: openbao `builtin/logical/pki/path_root.go::pathCAGenerateRoot`
    /// — the root CA may only be issued once per cave-runtime instance.
    #[error("root CA already exists")]
    RootAlreadyExists,
    /// Cite: openbao `builtin/logical/pki/path_intermediate.go` — an
    /// intermediate CA needs a parent issuer, or the request is a no-op.
    #[error("parent CA {0} not found")]
    ParentNotFound(String),
    /// Cite: RFC 5280 §6.1 — chain validation failure (untrusted issuer,
    /// expired cert, bad signature, etc).
    #[error("chain validation failed: {0}")]
    ChainInvalid(String),
    /// Cite: RFC 5280 §5 — revoked cert returned by CRL/OCSP responder.
    #[error("certificate {serial} revoked at {revoked_at}: {reason:?}")]
    Revoked {
        serial: String,
        revoked_at: String,
        reason: super::crl::RevocationReason,
    },
    /// cave multi-tenant invariant.
    #[error("cross-tenant access denied: store='{store}' request='{req}'")]
    CrossTenantDenied { store: String, req: String },
    #[error("invalid serial: {0}")]
    InvalidSerial(String),
    #[error("unsupported key algorithm: {0}")]
    UnsupportedKeyAlgorithm(String),
}
