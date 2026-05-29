// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Issuer backends — SelfSigned + CA.
//!
//! Cite: cert-manager v1.13.0
//! `pkg/issuer/selfsigned/issue.go` — SelfSigned issuer generates a key
//! pair and self-signs the certificate.
//! `pkg/issuer/ca/issue.go` — CA issuer signs leaf certs from a stored CA.
//!
//! cave uses `rcgen` (already in the workspace via cave-vault / cave-mesh)
//! for X.509 certificate generation.

use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, IsCa, KeyPair, SanType,
};
use time::{Duration, OffsetDateTime};
use thiserror::Error;

/// Input for an issuance operation (shared by SelfSigned + CA backends).
#[derive(Debug, Clone)]
pub struct IssueRequest {
    pub tenant_id: String,
    pub dns_names: Vec<String>,
    pub common_name: Option<String>,
    /// Duration in seconds.
    pub duration_seconds: i64,
    /// Cite: cert-manager `CertificateRequest.spec.isCA`.
    pub is_ca: bool,
}

/// Outcome of a successful issuance.
#[derive(Debug, Clone)]
pub struct IssueResult {
    /// PEM-encoded certificate (one or more blocks).
    pub certificate_pem: String,
    /// PEM-encoded private key (PKCS#8 EC or Ed25519).
    pub private_key_pem: String,
    /// Whether this is a CA certificate.
    pub is_ca: bool,
}

#[derive(Debug, Error)]
pub enum IssuerError {
    #[error("rcgen error: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("cross-tenant issuance denied: issuer belongs to '{issuer_tenant}', request from '{request_tenant}'")]
    CrossTenantDenied {
        issuer_tenant: String,
        request_tenant: String,
    },
    #[error("at least one of dns_names or common_name must be set")]
    NoDomains,
    #[error("invalid certificate PEM")]
    InvalidPem,
}

fn build_cert_params(req: &IssueRequest) -> Result<(CertificateParams, KeyPair), IssuerError> {
    let key_pair = KeyPair::generate()?;
    let mut params = CertificateParams::default();

    if let Some(cn) = &req.common_name {
        params
            .distinguished_name
            .push(DnType::CommonName, cn.as_str());
    }

    for dns in &req.dns_names {
        params
            .subject_alt_names
            .push(SanType::DnsName(dns.clone().try_into().map_err(|_| rcgen::Error::CouldNotParseCertificate)?));
    }

    // Cite: cert-manager duration handling. rcgen uses `time` crate.
    let now = OffsetDateTime::now_utc();
    let not_after = now + Duration::seconds(req.duration_seconds);
    params.not_before = now;
    params.not_after = not_after;

    if req.is_ca {
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    } else {
        params.is_ca = IsCa::NoCa;
    }

    Ok((params, key_pair))
}

/// Cite: cert-manager `pkg/issuer/selfsigned/issue.go` — generates a
/// new key pair and self-signs the certificate. The resulting cert's
/// issuer DN equals its subject DN.
pub struct SelfSignedIssuer {
    tenant_id: String,
}

impl SelfSignedIssuer {
    pub fn new(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
        }
    }

    /// Cite: cert-manager selfsigned issue — generate key + self-sign.
    pub fn issue(&self, req: &IssueRequest) -> Result<IssueResult, IssuerError> {
        if req.dns_names.is_empty() && req.common_name.is_none() {
            return Err(IssuerError::NoDomains);
        }
        let (params, key_pair) = build_cert_params(req)?;
        let private_key_pem = key_pair.serialize_pem();
        let cert = params.self_signed(&key_pair)?;
        Ok(IssueResult {
            certificate_pem: cert.pem(),
            private_key_pem,
            is_ca: req.is_ca,
        })
    }
}

/// Cite: cert-manager `pkg/issuer/ca/issue.go` — signs leaf certificates
/// using a stored CA certificate + private key. The CA cert is loaded from
/// PEM (mirrors cert-manager's `ca.secretName` Secret lookup).
pub struct CaIssuer {
    tenant_id: String,
    ca_cert: Certificate,
    ca_key_pair: KeyPair,
}

impl CaIssuer {
    /// Construct a `CaIssuer` from PEM-encoded CA cert + key.
    /// Cite: cert-manager `pkg/issuer/ca/setup.go::Setup` — load the CA
    /// from the configured Secret.
    pub fn from_pem(
        tenant_id: impl Into<String>,
        cert_pem: &str,
        key_pem: &str,
    ) -> Result<Self, IssuerError> {
        let ca_key_pair = KeyPair::from_pem(key_pem)?;
        let ca_cert_params = CertificateParams::from_ca_cert_pem(cert_pem)?;
        let ca_cert = ca_cert_params.self_signed(&ca_key_pair)?;
        Ok(Self {
            tenant_id: tenant_id.into(),
            ca_cert,
            ca_key_pair,
        })
    }

    /// Cite: cert-manager `pkg/issuer/ca/issue.go::Issue` — sign a leaf
    /// cert with the CA key. Returns the leaf cert PEM + new private key PEM.
    pub fn sign(&self, req: &IssueRequest) -> Result<IssueResult, IssuerError> {
        if req.tenant_id != self.tenant_id {
            return Err(IssuerError::CrossTenantDenied {
                issuer_tenant: self.tenant_id.clone(),
                request_tenant: req.tenant_id.clone(),
            });
        }
        if req.dns_names.is_empty() && req.common_name.is_none() {
            return Err(IssuerError::NoDomains);
        }
        let (params, key_pair) = build_cert_params(req)?;
        let private_key_pem = key_pair.serialize_pem();
        let leaf_cert = params.signed_by(&key_pair, &self.ca_cert, &self.ca_key_pair)?;
        Ok(IssueResult {
            certificate_pem: leaf_cert.pem(),
            private_key_pem,
            is_ca: req.is_ca,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selfsigned_produces_cert_pem() {
        let issuer = SelfSignedIssuer::new("t1");
        let req = IssueRequest {
            tenant_id: "t1".into(),
            dns_names: vec!["test.example.com".into()],
            common_name: Some("test.example.com".into()),
            duration_seconds: 90 * 86_400,
            is_ca: false,
        };
        let result = issuer.issue(&req).unwrap();
        assert!(result.certificate_pem.contains("CERTIFICATE"));
        assert!(!result.is_ca);
    }

    #[test]
    fn selfsigned_ca_sets_is_ca_true() {
        let issuer = SelfSignedIssuer::new("t1");
        let req = IssueRequest {
            tenant_id: "t1".into(),
            dns_names: vec![],
            common_name: Some("Internal CA".into()),
            duration_seconds: 365 * 86_400,
            is_ca: true,
        };
        let result = issuer.issue(&req).unwrap();
        assert!(result.is_ca);
    }

    #[test]
    fn ca_issuer_rejects_cross_tenant() {
        let ss = SelfSignedIssuer::new("t1");
        let ca_req = IssueRequest {
            tenant_id: "t1".into(),
            dns_names: vec![],
            common_name: Some("Test CA".into()),
            duration_seconds: 365 * 86_400,
            is_ca: true,
        };
        let ca = ss.issue(&ca_req).unwrap();
        let issuer = CaIssuer::from_pem("t1", &ca.certificate_pem, &ca.private_key_pem).unwrap();
        let leaf = IssueRequest {
            tenant_id: "t2".into(), // wrong tenant
            dns_names: vec!["api.example.com".into()],
            common_name: None,
            duration_seconds: 90 * 86_400,
            is_ca: false,
        };
        assert!(issuer.sign(&leaf).is_err());
    }
}
