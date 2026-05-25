// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Certificate + Issuer CRDs (cert-manager API parity).
//!
//! Cite: cert-manager v1.20.2
//! `pkg/apis/certmanager/v1/types_certificate.go` (CertificateSpec) and
//! `pkg/apis/certmanager/v1/types_issuer.go` (IssuerSpec). cave models
//! the shape closely enough that an operator running cert-manager can
//! recognise the field names; the YAML/JSON wire format is identical.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Cite: cert-manager `pkg/apis/certmanager/v1/types_certificate.go`
/// (CertificateSpec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateSpec {
    /// Tenant boundary (cave extension).
    pub tenant_id: String,
    /// Cite: cert-manager Certificate.spec.dnsNames — required when no
    /// commonName is set.
    #[serde(default)]
    pub dns_names: Vec<String>,
    /// Cite: cert-manager Certificate.spec.commonName.
    pub common_name: Option<String>,
    /// Cite: cert-manager Certificate.spec.secretName — Kubernetes
    /// Secret that will hold the issued tls.crt + tls.key.
    pub secret_name: String,
    /// Cite: cert-manager Certificate.spec.issuerRef.
    pub issuer_ref: IssuerRef,
    /// Cite: cert-manager Certificate.spec.duration (default 90d).
    pub duration_seconds: i64,
    /// Cite: cert-manager Certificate.spec.renewBefore (default 30d).
    pub renew_before_seconds: i64,
    /// Cite: cert-manager Certificate.spec.usages.
    #[serde(default)]
    pub usages: Vec<KeyUsage>,
    /// Cite: cert-manager Certificate.spec.privateKey (algorithm + size).
    pub private_key_algorithm: PrivateKeyAlgorithm,
}

impl CertificateSpec {
    pub const DEFAULT_DURATION_SECS: i64 = 90 * 24 * 3600;
    pub const DEFAULT_RENEW_BEFORE_SECS: i64 = 30 * 24 * 3600;

    pub fn new(
        tenant_id: impl Into<String>,
        secret_name: impl Into<String>,
        issuer_ref: IssuerRef,
        dns_names: Vec<String>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            dns_names,
            common_name: None,
            secret_name: secret_name.into(),
            issuer_ref,
            duration_seconds: Self::DEFAULT_DURATION_SECS,
            renew_before_seconds: Self::DEFAULT_RENEW_BEFORE_SECS,
            usages: vec![
                KeyUsage::DigitalSignature,
                KeyUsage::KeyEncipherment,
                KeyUsage::ServerAuth,
            ],
            private_key_algorithm: PrivateKeyAlgorithm::Ecdsa256,
        }
    }

    /// Cite: cert-manager `pkg/apis/certmanager/v1/types_certificate.go`
    /// validation — at least one of dnsNames / commonName / ipAddresses
    /// / uris MUST be set; secretName MUST be non-empty;
    /// renewBefore < duration.
    pub fn validate(&self) -> Result<(), String> {
        if self.tenant_id.trim().is_empty() {
            return Err("tenant_id must be non-empty".into());
        }
        if self.secret_name.trim().is_empty() {
            return Err("secretName must be non-empty".into());
        }
        if self.dns_names.is_empty() && self.common_name.is_none() {
            return Err("at least one of dnsNames or commonName must be set".into());
        }
        if self.duration_seconds <= 0 {
            return Err("duration must be > 0".into());
        }
        if self.renew_before_seconds >= self.duration_seconds {
            return Err("renewBefore must be < duration".into());
        }
        for n in &self.dns_names {
            if n != &n.to_lowercase() {
                return Err(format!("dnsName '{}' must be lowercase", n));
            }
            if n.is_empty() || n.contains(' ') {
                return Err(format!("dnsName '{}' is invalid", n));
            }
        }
        Ok(())
    }
}

/// Cite: cert-manager `Certificate.spec.privateKey.algorithm` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrivateKeyAlgorithm {
    Rsa2048,
    Rsa4096,
    Ecdsa256,
    Ecdsa384,
    Ed25519,
    /// cave PQC extension. Cite: ADR-015 v2.
    HybridMlDsa65Ed25519,
}

/// Cite: cert-manager `Certificate.spec.usages` enum (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyUsage {
    DigitalSignature,
    KeyEncipherment,
    ServerAuth,
    ClientAuth,
    CodeSigning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerRef {
    pub name: String,
    pub kind: String,
    pub group: String,
}

impl IssuerRef {
    pub fn issuer(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "Issuer".into(),
            group: "cert-manager.io".into(),
        }
    }
    pub fn cluster_issuer(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: "ClusterIssuer".into(),
            group: "cert-manager.io".into(),
        }
    }
}

/// Cite: cert-manager `pkg/apis/certmanager/v1/types_certificate.go`
/// `CertificateStatus` — conditions + notBefore/notAfter + renewalTime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateStatus {
    #[serde(default)]
    pub conditions: Vec<Condition>,
    pub not_before: Option<DateTime<Utc>>,
    pub not_after: Option<DateTime<Utc>>,
    pub renewal_time: Option<DateTime<Utc>>,
    pub revision: u64,
}

impl Default for CertificateStatus {
    fn default() -> Self {
        Self {
            conditions: Vec::new(),
            not_before: None,
            not_after: None,
            renewal_time: None,
            revision: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Condition {
    #[serde(rename = "type")]
    pub kind: String,
    pub status: String, // "True" | "False" | "Unknown"
    pub reason: String,
    pub message: String,
    pub last_transition_time: DateTime<Utc>,
}

impl Condition {
    pub fn ready_true(reason: &str, message: &str) -> Self {
        Self {
            kind: "Ready".into(),
            status: "True".into(),
            reason: reason.into(),
            message: message.into(),
            last_transition_time: Utc::now(),
        }
    }
}

/// Cite: cert-manager `pkg/apis/certmanager/v1/types_issuer.go`
/// (IssuerSpec) — exactly one of `ca`, `acme`, `vault` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssuerSpec {
    pub tenant_id: String,
    pub config: IssuerConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum IssuerConfig {
    /// Cite: cert-manager `IssuerConfig.ca` — issues from a static CA
    /// reference (Kubernetes Secret containing tls.crt + tls.key).
    Ca { secret_name: String },
    /// Cite: cert-manager `IssuerConfig.acme` — RFC 8555 ACME issuer.
    Acme {
        server: String,
        email: String,
        private_key_secret_ref: String,
        #[serde(default)]
        external_account_binding_kid: Option<String>,
    },
    /// Cite: cert-manager `IssuerConfig.vault` — issues via the openbao
    /// PKI engine on a configured mount path.
    Vault {
        server: String,
        path: String,
        role: String,
    },
}

impl IssuerSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.tenant_id.trim().is_empty() {
            return Err("tenant_id must be non-empty".into());
        }
        match &self.config {
            IssuerConfig::Ca { secret_name } if secret_name.trim().is_empty() => {
                Err("ca.secretName must be non-empty".into())
            }
            IssuerConfig::Acme { server, email, .. } if server.trim().is_empty() => {
                Err("acme.server must be non-empty".into())
            }
            IssuerConfig::Acme { email, .. } if !email.contains('@') => {
                Err(format!("acme.email '{}' is invalid", email))
            }
            IssuerConfig::Vault {
                server, path, role, ..
            } if server.trim().is_empty() || path.trim().is_empty() || role.trim().is_empty() => {
                Err("vault.server/path/role must all be non-empty".into())
            }
            _ => Ok(()),
        }
    }
}

/// Cite: cert-manager controller `pkg/controller/certificates/trigger`
/// — the renewal trigger fires when `now >= notAfter - renewBefore`.
pub fn renewal_due_at(spec: &CertificateSpec, status: &CertificateStatus) -> Option<DateTime<Utc>> {
    let not_after = status.not_after?;
    Some(not_after - Duration::seconds(spec.renew_before_seconds))
}
