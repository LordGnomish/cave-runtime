// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("registration entry not found: {0}")]
    EntryNotFound(String),
    #[error("attestation failed: {0}")]
    AttestationFailed(String),
    #[error("svid issuance failed: {0}")]
    SvidIssuanceFailed(String),
    #[error("federation bundle invalid: {0}")]
    FederationInvalid(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, IdentityError>;
