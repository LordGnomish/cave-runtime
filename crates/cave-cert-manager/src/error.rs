// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Error surface — mirrors cert-manager's `cmerrors` package.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CertManagerError {
    #[error("certificate not found: {0}")]
    CertificateNotFound(String),

    #[error("issuer not found: {0}")]
    IssuerNotFound(String),

    #[error("cluster-issuer not found: {0}")]
    ClusterIssuerNotFound(String),

    #[error("certificate request not found: {0}")]
    CertificateRequestNotFound(String),

    #[error("invalid spec: {0}")]
    InvalidSpec(String),

    #[error("dnsNames must contain at least one entry")]
    EmptyDnsNames,

    #[error("invalid dnsName {name}: {reason}")]
    InvalidDnsName { name: String, reason: String },

    #[error("renewBefore ({renew_before_seconds}s) >= duration ({duration_seconds}s)")]
    RenewBeforeExceedsDuration {
        renew_before_seconds: i64,
        duration_seconds: i64,
    },

    #[error("ACME order failed: {0}")]
    AcmeOrder(String),

    #[error("ACME challenge {challenge_id} {reason}")]
    AcmeChallenge { challenge_id: String, reason: String },

    #[error("ACME backend error: {0}")]
    Acme(#[from] cave_acme::AcmeError),

    #[error("PKI backend error: {0}")]
    Pki(#[from] cave_pki::PkiError),

    #[error("vault issuer missing keychain handle: {handle}")]
    VaultKeychainMissing { handle: String },

    #[error("vault issuer keychain handle must use keychain: scheme — got `{0}`")]
    VaultKeychainScheme(String),

    #[error(
        "cross-tenant denied — request from `{request_tenant}` for resource owned by `{owner_tenant}`"
    )]
    CrossTenantDenied {
        owner_tenant: String,
        request_tenant: String,
    },

    #[error("secret not found: {0}")]
    SecretNotFound(String),

    #[error("certificate {0} not yet ready")]
    NotReady(String),
}

pub type CertManagerResult<T> = Result<T, CertManagerError>;
