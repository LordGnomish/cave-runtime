// SPDX-License-Identifier: AGPL-3.0-or-later
use thiserror::Error;

pub type AcmeResult<T> = Result<T, AcmeError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AcmeError {
    /// Cite: RFC 8555 §6.7 problem document `urn:ietf:params:acme:error:accountDoesNotExist`.
    #[error("account {0} does not exist")]
    AccountNotFound(String),
    /// Cite: RFC 8555 §6.7 `urn:ietf:params:acme:error:malformed`.
    #[error("malformed request: {0}")]
    Malformed(String),
    /// Cite: RFC 8555 §6.7 `urn:ietf:params:acme:error:unauthorized`.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// Cite: RFC 8555 §7.4 — order finalize must produce a valid CSR.
    #[error("order {0} not ready for finalize (status: {1})")]
    OrderNotReady(String, String),
    /// Cite: RFC 8555 §8 challenge validation failures map to invalid.
    #[error("challenge {0} invalid: {1}")]
    ChallengeInvalid(String, String),
    /// cave-acme multi-tenant invariant.
    #[error("cross-tenant access denied: store='{store}' request='{req}'")]
    CrossTenantDenied { store: String, req: String },
}
