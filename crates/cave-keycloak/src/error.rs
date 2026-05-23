// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Error type for the Keycloak control plane reimplementation.

use thiserror::Error;

/// All cave-keycloak failure modes. Constructors carry enough structured
/// context for cave-oncall to correlate (tenant_id, realm_id, user_id,
/// client_id) without an additional log scrape.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeycloakError {
    #[error("realm not found: {0}")]
    RealmNotFound(String),

    #[error("realm already exists: {0}")]
    RealmExists(String),

    #[error("user not found: {0}")]
    UserNotFound(String),

    #[error("user already exists: {0}")]
    UserExists(String),

    #[error("client not found: {0}")]
    ClientNotFound(String),

    #[error("role not found: {0}")]
    RoleNotFound(String),

    #[error("group not found: {0}")]
    GroupNotFound(String),

    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("credential locked (brute-force protection): account_id={account_id} retry_after_seconds={retry_after_seconds}")]
    CredentialLocked {
        account_id: String,
        retry_after_seconds: u64,
    },

    #[error("password policy violation: {0}")]
    PasswordPolicyViolation(String),

    #[error("invalid client_id or redirect_uri")]
    InvalidClientOrRedirect,

    #[error("invalid grant: {0}")]
    InvalidGrant(String),

    #[error("token expired")]
    TokenExpired,

    #[error("token signature invalid")]
    TokenSignatureInvalid,

    #[error("token revoked")]
    TokenRevoked,

    #[error("PKCE verification failed: {0}")]
    PkceFailed(String),

    #[error("scope not permitted: {0}")]
    ScopeNotPermitted(String),

    #[error("cross-tenant access denied (owner={owner_tenant} requester={request_tenant})")]
    CrossTenantDenied {
        owner_tenant: String,
        request_tenant: String,
    },

    #[error("SAML response invalid: {0}")]
    SamlInvalid(String),

    #[error("LDAP error: {0}")]
    LdapError(String),

    #[error("identity provider error: {0}")]
    BrokeringError(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, KeycloakError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_tenant_message_surfaces_both_ids() {
        let e = KeycloakError::CrossTenantDenied {
            owner_tenant: "t-1".into(),
            request_tenant: "t-2".into(),
        };
        let msg = format!("{}", e);
        assert!(msg.contains("t-1"));
        assert!(msg.contains("t-2"));
    }

    #[test]
    fn credential_locked_message_carries_retry_after() {
        let e = KeycloakError::CredentialLocked {
            account_id: "u-9".into(),
            retry_after_seconds: 300,
        };
        let msg = format!("{}", e);
        assert!(msg.contains("u-9"));
        assert!(msg.contains("300"));
    }

    #[test]
    fn pkce_failure_carries_reason() {
        let e = KeycloakError::PkceFailed("verifier_too_short".into());
        assert!(format!("{}", e).contains("verifier_too_short"));
    }
}
