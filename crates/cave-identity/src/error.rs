// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("registration entry not found: {0}")]
    EntryNotFound(String),
    #[error("registration entry exists: {0}")]
    EntryExists(String),
    #[error("invalid spiffe id: {0}")]
    InvalidSpiffeId(String),
    #[error("invalid trust domain: {0}")]
    InvalidTrustDomain(String),
    #[error("attestation failed: {0}")]
    AttestationFailed(String),
    #[error("attestor not found: {0}")]
    AttestorNotFound(String),
    #[error("svid issuance failed: {0}")]
    SvidIssuanceFailed(String),
    #[error("svid verification failed: {0}")]
    SvidVerificationFailed(String),
    #[error("jwt invalid: {0}")]
    JwtInvalid(String),
    #[error("federation bundle invalid: {0}")]
    FederationInvalid(String),
    #[error("federation endpoint unreachable: {0}")]
    FederationUnreachable(String),
    #[error("bundle not found: {0}")]
    BundleNotFound(String),
    #[error("ca not initialised")]
    CaNotInitialised,
    #[error("policy violation: {0}")]
    PolicyViolation(String),
    #[error("ttl out of bounds: requested={requested} min={min} max={max}")]
    TtlOutOfBounds { requested: u32, min: u32, max: u32 },
    #[error("agent banned: {0}")]
    AgentBanned(String),
    #[error("oidc invalid: {0}")]
    OidcInvalid(String),
    #[error("io: {0}")]
    Io(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, IdentityError>;

impl From<serde_json::Error> for IdentityError {
    fn from(e: serde_json::Error) -> Self {
        IdentityError::Internal(format!("serde_json: {}", e))
    }
}

impl From<std::io::Error> for IdentityError {
    fn from(e: std::io::Error) -> Self {
        IdentityError::Io(e.to_string())
    }
}
