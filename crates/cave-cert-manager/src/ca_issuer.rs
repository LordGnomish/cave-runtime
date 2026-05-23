// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CA issuer — signs against an internal CA hierarchy managed by
//! [`cave_pki`].
//!
//! Cite: `pkg/issuer/ca/sign.go::Sign` — cert-manager's CA issuer loads
//! the named secret (cert + key) and signs the CSR. cave-cert-manager
//! delegates the hierarchy management to cave-pki's three-tier
//! Root → Platform Intermediate → per-tenant Intermediate model and
//! resolves `spec.secretName` onto the tenant intermediate serial.

use crate::error::{CertManagerError, CertManagerResult};
use crate::issuer::IssueOutcome;
use crate::models::{CertificateRequest, IssuerSpec};
use crate::selfsigned_issuer::build_synthetic_pem;
use cave_pki::{Ca, CaKind, KeyAlgorithm};
use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};

#[derive(Debug, Default)]
pub struct CaIssuer {
    pub ca: Ca,
    pub root_serial: Option<String>,
    pub platform_serial: Option<String>,
}

impl CaIssuer {
    /// Lazy initialisation — the first issuance bootstraps the
    /// Root + Platform Intermediate. Subsequent calls reuse them.
    /// Tenants get one intermediate per `tenant_id` (cave invariant).
    pub fn issue(
        &mut self,
        spec: &IssuerSpec,
        req: &CertificateRequest,
    ) -> CertManagerResult<IssueOutcome> {
        let (secret_name, crl_dps) = match spec {
            IssuerSpec::Ca {
                secret_name,
                crl_distribution_points,
            } => (secret_name.clone(), crl_distribution_points.clone()),
            _ => {
                return Err(CertManagerError::InvalidSpec(
                    "CaIssuer.issue called with non-Ca spec".into(),
                ));
            }
        };

        if self.root_serial.is_none() {
            let root = self
                .ca
                .generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 30)?;
            self.root_serial = Some(root);
        }
        if self.platform_serial.is_none() {
            let plat = self
                .ca
                .generate_platform_intermediate("Cave Platform Intermediate", KeyAlgorithm::EcdsaP256)?;
            self.platform_serial = Some(plat);
        }
        let tenant_serial = self
            .ca
            .generate_tenant_intermediate(&req.tenant_id, KeyAlgorithm::EcdsaP256)?;

        let tenant_handle = self
            .ca
            .handle(&tenant_serial)
            .ok_or_else(|| CertManagerError::IssuerNotFound(tenant_serial.clone()))?
            .clone();
        if tenant_handle.kind != CaKind::TenantIntermediate {
            return Err(CertManagerError::InvalidSpec(format!(
                "tenant serial {} is not a tenant intermediate",
                tenant_serial
            )));
        }

        // Leaf serial = sha256(tenant|secret|revision).
        let mut hasher = Sha256::new();
        hasher.update(req.tenant_id.as_bytes());
        hasher.update(secret_name.as_bytes());
        hasher.update(req.revision.to_be_bytes());
        let leaf_serial = hex::encode(hasher.finalize()).chars().take(32).collect::<String>();

        let now = Utc::now();
        let not_after = now + Duration::seconds(req.duration_seconds);

        let leaf_pem = build_synthetic_pem(
            "CA-ISSUED-LEAF",
            &req.name,
            &req.dns_names,
            &leaf_serial,
            &crl_dps,
            req.is_ca,
        );
        let tenant_pem = build_synthetic_pem(
            "CA-TENANT-INTERMEDIATE",
            &tenant_handle.subject_common_name,
            &[],
            &tenant_serial,
            &crl_dps,
            true,
        );
        let chain_pem = format!("{}{}", leaf_pem, tenant_pem);
        // CA bundle is the tenant intermediate + the platform intermediate
        // chain — we drop the root from the bundle because clients pin
        // the root out-of-band (cave-pki distribution).
        let mut ca_pem = tenant_pem.clone();
        if let Some(plat) = self.ca.handle(self.platform_serial.as_deref().unwrap_or("")) {
            ca_pem.push_str(&build_synthetic_pem(
                "CA-PLATFORM-INTERMEDIATE",
                &plat.subject_common_name,
                &[],
                &plat.serial,
                &crl_dps,
                true,
            ));
        }

        Ok(IssueOutcome {
            certificate_chain_pem: chain_pem,
            ca_pem,
            not_before: now,
            not_after,
            serial: leaf_serial,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateRequestStatus, IssuerRef, IssuerRefKind, Usage,
    };
    use uuid::Uuid;

    fn cert_req(tenant: &str, name: &str, revision: u64) -> CertificateRequest {
        CertificateRequest {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            tenant_id: tenant.into(),
            certificate_id: Uuid::new_v4(),
            revision,
            issuer_ref: IssuerRef {
                name: "ca".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            usages: vec![Usage::ServerAuth],
            dns_names: vec![format!("{}.example.com", name)],
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

    fn ca_spec() -> IssuerSpec {
        IssuerSpec::Ca {
            secret_name: "cave-ca".into(),
            crl_distribution_points: vec![],
        }
    }

    #[test]
    fn first_issuance_bootstraps_root_and_platform() {
        let mut issuer = CaIssuer::default();
        let _ = issuer.issue(&ca_spec(), &cert_req("t-1", "svc", 1)).unwrap();
        assert!(issuer.root_serial.is_some());
        assert!(issuer.platform_serial.is_some());
        assert!(issuer.ca.tenant_serial("t-1").is_some());
    }

    #[test]
    fn second_issuance_reuses_tenant_intermediate() {
        let mut issuer = CaIssuer::default();
        let _ = issuer.issue(&ca_spec(), &cert_req("t-1", "svc-a", 1)).unwrap();
        let initial_count = issuer.ca.tenant_count();
        let _ = issuer.issue(&ca_spec(), &cert_req("t-1", "svc-b", 1)).unwrap();
        assert_eq!(issuer.ca.tenant_count(), initial_count);
    }

    #[test]
    fn distinct_tenants_get_distinct_intermediates() {
        let mut issuer = CaIssuer::default();
        let _ = issuer.issue(&ca_spec(), &cert_req("t-1", "svc", 1)).unwrap();
        let _ = issuer.issue(&ca_spec(), &cert_req("t-2", "svc", 1)).unwrap();
        let s1 = issuer.ca.tenant_serial("t-1").unwrap().to_string();
        let s2 = issuer.ca.tenant_serial("t-2").unwrap().to_string();
        assert_ne!(s1, s2);
    }

    #[test]
    fn chain_carries_tenant_intermediate() {
        let mut issuer = CaIssuer::default();
        let outcome = issuer.issue(&ca_spec(), &cert_req("t-1", "svc", 1)).unwrap();
        assert!(
            outcome
                .certificate_chain_pem
                .matches("BEGIN CERTIFICATE")
                .count()
                >= 2,
            "chain must include leaf + intermediate"
        );
        assert!(outcome.ca_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn rejects_wrong_spec() {
        let mut issuer = CaIssuer::default();
        let err = issuer
            .issue(
                &IssuerSpec::SelfSigned {
                    crl_distribution_points: vec![],
                },
                &cert_req("t-1", "svc", 1),
            )
            .unwrap_err();
        assert!(matches!(err, CertManagerError::InvalidSpec(_)));
    }
}
