// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Issuer registry — dispatches a `CertificateRequest` to the backend
//! matching the spec's `IssuerSpec` variant.
//!
//! Cite: `pkg/controller/issuers/sync.go` — cert-manager reads
//! `Issuer.spec.<kind>` once at reconcile and forwards to the matching
//! backend.

use crate::acme_issuer::AcmeIssuer;
use crate::ca_issuer::CaIssuer;
use crate::error::{CertManagerError, CertManagerResult};
use crate::models::{CertificateRequest, IssuerKind, IssuerSpec};
use crate::selfsigned_issuer::SelfSignedIssuer;
use crate::vault_issuer::VaultIssuer;
use chrono::{DateTime, Utc};

/// One successful issuance — leaf-first chain PEM + CA chain PEM +
/// validity window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueOutcome {
    pub certificate_chain_pem: String,
    pub ca_pem: String,
    pub not_before: DateTime<Utc>,
    pub not_after: DateTime<Utc>,
    pub serial: String,
}

/// Routing-only struct — instances are stateless per request.
#[derive(Debug, Default)]
pub struct IssuerRegistry {
    pub acme: AcmeIssuer,
    pub ca: CaIssuer,
    pub vault: VaultIssuer,
    pub self_signed: SelfSignedIssuer,
}

impl IssuerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Dispatch to the right backend. The ACME backend takes
    /// `&mut self` because solving runs the ACME order forward in
    /// memory.
    pub fn issue(
        &mut self,
        spec: &IssuerSpec,
        req: &CertificateRequest,
    ) -> CertManagerResult<IssueOutcome> {
        match spec {
            IssuerSpec::Acme { .. } => self.acme.issue(spec, req),
            IssuerSpec::Ca { .. } => self.ca.issue(spec, req),
            IssuerSpec::Vault { .. } => self.vault.issue(spec, req),
            IssuerSpec::SelfSigned { .. } => self.self_signed.issue(spec, req),
            IssuerSpec::Venafi { .. } => Err(CertManagerError::InvalidSpec(
                "Venafi issuer runtime is Phase 2 — see [[partial]] venafi-issuer".into(),
            )),
        }
    }

    pub fn supports(&self, kind: IssuerKind) -> bool {
        matches!(
            kind,
            IssuerKind::Acme | IssuerKind::Ca | IssuerKind::Vault | IssuerKind::SelfSigned
        )
    }
}

/// Helper used by ACME + Vault issuers — strip the `keychain:` scheme
/// off a keychain handle. cave-cert-manager NEVER stores secret
/// material in process memory.
pub(crate) fn require_keychain_handle(handle: &str) -> CertManagerResult<&str> {
    handle
        .strip_prefix("keychain:")
        .ok_or_else(|| CertManagerError::VaultKeychainScheme(handle.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AcmeChallengeSolver, AcmeSolver, CertificateRequestStatus, DnsProvider, IssuerRef,
        IssuerRefKind, Usage,
    };
    use uuid::Uuid;

    fn cert_req() -> CertificateRequest {
        CertificateRequest {
            id: Uuid::new_v4(),
            name: "demo-1".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            certificate_id: Uuid::new_v4(),
            revision: 1,
            issuer_ref: IssuerRef {
                name: "selfsigned".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            usages: vec![Usage::ServerAuth],
            dns_names: vec!["example.com".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 7 * 24 * 3600,
            is_ca: false,
            created_at: Utc::now(),
            status: CertificateRequestStatus::default(),
        }
    }

    #[test]
    fn keychain_handle_must_use_scheme() {
        assert!(require_keychain_handle("plaintext").is_err());
        assert_eq!(require_keychain_handle("keychain:cave-acme-key").unwrap(), "cave-acme-key");
    }

    #[test]
    fn registry_supports_four_backends() {
        let r = IssuerRegistry::new();
        assert!(r.supports(IssuerKind::Acme));
        assert!(r.supports(IssuerKind::Ca));
        assert!(r.supports(IssuerKind::Vault));
        assert!(r.supports(IssuerKind::SelfSigned));
        assert!(!r.supports(IssuerKind::Venafi));
    }

    #[test]
    fn registry_routes_selfsigned() {
        let mut r = IssuerRegistry::new();
        let req = cert_req();
        let outcome = r
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &req,
            )
            .unwrap();
        assert!(outcome.certificate_chain_pem.contains("BEGIN CERTIFICATE"));
        assert!(outcome.not_after > outcome.not_before);
    }

    #[test]
    fn registry_rejects_venafi_runtime() {
        let mut r = IssuerRegistry::new();
        let req = cert_req();
        let err = r
            .issue(
                &IssuerSpec::Venafi {
                    zone: "ops".into(),
                    token_keychain_handle: "keychain:venafi-token".into(),
                },
                &req,
            )
            .unwrap_err();
        assert!(matches!(err, CertManagerError::InvalidSpec(_)));
    }

    #[test]
    fn registry_routes_acme_solver_path() {
        let mut r = IssuerRegistry::new();
        let req = cert_req();
        let spec = IssuerSpec::Acme {
            directory_url: "https://acme.example.com/directory".into(),
            account_key_keychain_handle: "keychain:cave-acme-key".into(),
            email: vec!["ops@example.com".into()],
            terms_of_service_agreed: true,
            solvers: vec![AcmeSolver {
                dns_zones: vec![],
                challenge: AcmeChallengeSolver::Dns01 {
                    provider: DnsProvider::CaveDns {
                        zone: "example.com.".into(),
                    },
                },
            }],
        };
        let outcome = r.issue(&spec, &req).unwrap();
        assert!(!outcome.serial.is_empty());
    }
}
