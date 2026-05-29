// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! PKCS#10 CSR generation.
//!
//! Cite: cert-manager v1.13.0 `pkg/util/pki/csr.go::GenerateCSR` — given
//! a set of DNS names and an optional common name, generate a key pair and
//! produce a PKCS#10 CSR PEM.
//!
//! cave uses `rcgen` (already in the workspace via cave-vault / cave-mesh)
//! which provides a safe, pure-Rust X.509 / CSR implementation.

use rcgen::{CertificateParams, DnType, KeyPair, SanType};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CsrError {
    #[error("rcgen error: {0}")]
    Rcgen(#[from] rcgen::Error),
    #[error("tenant_id must be non-empty")]
    EmptyTenant,
    #[error("at least one of dns_names or common_name must be set")]
    NoDomains,
}

/// Input parameters for CSR generation.
///
/// Cite: cert-manager `pkg/util/pki/csr.go` fields.
#[derive(Debug, Clone)]
pub struct CsrParams {
    pub tenant_id: String,
    pub dns_names: Vec<String>,
    /// Cite: cert-manager `CertificateSpec.commonName`.
    pub common_name: Option<String>,
}

/// Output of `CsrBuilder::build` — the CSR PEM + corresponding private key
/// PEM. The private key is handed back to the caller for storage in the
/// `secret_name` Kubernetes Secret (or cave-certs CertificateStore).
pub struct CsrOutput {
    /// PKCS#10 PEM (`-----BEGIN CERTIFICATE REQUEST-----`).
    pub pem: String,
    /// EC (P-256 PKCS#8) private key PEM (`-----BEGIN PRIVATE KEY-----`).
    pub private_key_pem: String,
}

pub struct CsrBuilder;

impl CsrBuilder {
    /// Cite: cert-manager `pkg/util/pki/csr.go::GenerateCSR` —
    /// build a PKCS#10 CSR for the given names.
    pub fn build(params: &CsrParams) -> Result<CsrOutput, CsrError> {
        if params.tenant_id.trim().is_empty() {
            return Err(CsrError::EmptyTenant);
        }
        if params.dns_names.is_empty() && params.common_name.is_none() {
            return Err(CsrError::NoDomains);
        }

        let key_pair = KeyPair::generate()?;
        let private_key_pem = key_pair.serialize_pem();

        let mut cert_params = CertificateParams::default();

        // Cite: cert-manager commonName handling.
        if let Some(cn) = &params.common_name {
            cert_params
                .distinguished_name
                .push(DnType::CommonName, cn.as_str());
        }

        // Cite: RFC 5280 §4.2.1.6 — Subject Alternative Names.
        for dns in &params.dns_names {
            cert_params
                .subject_alt_names
                .push(SanType::DnsName(dns.clone().try_into().map_err(|_| rcgen::Error::CouldNotParseCertificate)?));
        }

        let csr = cert_params.serialize_request(&key_pair)?;
        let pem = csr.pem()?;

        Ok(CsrOutput {
            pem,
            private_key_pem,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_produces_csr_pem() {
        let params = CsrParams {
            tenant_id: "t1".into(),
            dns_names: vec!["api.example.com".into()],
            common_name: Some("api.example.com".into()),
        };
        let out = CsrBuilder::build(&params).unwrap();
        assert!(out.pem.contains("CERTIFICATE REQUEST"));
        assert!(!out.private_key_pem.is_empty());
    }

    #[test]
    fn build_rejects_empty_tenant() {
        let params = CsrParams {
            tenant_id: "".into(),
            dns_names: vec!["api.example.com".into()],
            common_name: None,
        };
        assert!(CsrBuilder::build(&params).is_err());
    }

    #[test]
    fn build_rejects_no_domains() {
        let params = CsrParams {
            tenant_id: "t1".into(),
            dns_names: vec![],
            common_name: None,
        };
        assert!(CsrBuilder::build(&params).is_err());
    }
}
