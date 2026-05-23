// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! In-memory store — same shape as cert-manager's cache (every CRD has
//! an indexer keyed by `namespace/name`). Tenant-scoped: every method
//! takes a `tenant_id` and returns
//! [`CertManagerError::CrossTenantDenied`] when the lookup crosses
//! tenant boundaries.
//!
//! Cite: `pkg/controller/util/store.go` — cert-manager's `cmlister`
//! interfaces are namespaced reads + a `List` per resource.

use crate::error::{CertManagerError, CertManagerResult};
use crate::models::{
    Certificate, CertificateRequest, ClusterIssuer, IssuerResource,
};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct CertManagerStore {
    certificates: HashMap<Uuid, Certificate>,
    requests: HashMap<Uuid, CertificateRequest>,
    issuers: HashMap<Uuid, IssuerResource>,
    cluster_issuers: HashMap<Uuid, ClusterIssuer>,
}

impl CertManagerStore {
    pub fn new() -> Self {
        Self::default()
    }

    // ── Certificate ────────────────────────────────────────────────────

    pub fn put_certificate(&mut self, cert: Certificate) -> Uuid {
        let id = cert.id;
        self.certificates.insert(id, cert);
        id
    }

    pub fn certificate(
        &self,
        tenant_id: &str,
        id: Uuid,
    ) -> CertManagerResult<&Certificate> {
        let c = self
            .certificates
            .get(&id)
            .ok_or_else(|| CertManagerError::CertificateNotFound(id.to_string()))?;
        check_tenant(&c.tenant_id, tenant_id)?;
        Ok(c)
    }

    pub fn certificate_mut(
        &mut self,
        tenant_id: &str,
        id: Uuid,
    ) -> CertManagerResult<&mut Certificate> {
        let c = self
            .certificates
            .get_mut(&id)
            .ok_or_else(|| CertManagerError::CertificateNotFound(id.to_string()))?;
        check_tenant(&c.tenant_id, tenant_id)?;
        Ok(c)
    }

    pub fn list_certificates(&self, tenant_id: &str) -> Vec<&Certificate> {
        self.certificates
            .values()
            .filter(|c| c.tenant_id == tenant_id)
            .collect()
    }

    pub fn certificate_count(&self) -> usize {
        self.certificates.len()
    }

    // ── CertificateRequest ─────────────────────────────────────────────

    pub fn put_request(&mut self, req: CertificateRequest) -> Uuid {
        let id = req.id;
        self.requests.insert(id, req);
        id
    }

    pub fn request(
        &self,
        tenant_id: &str,
        id: Uuid,
    ) -> CertManagerResult<&CertificateRequest> {
        let r = self
            .requests
            .get(&id)
            .ok_or_else(|| CertManagerError::CertificateRequestNotFound(id.to_string()))?;
        check_tenant(&r.tenant_id, tenant_id)?;
        Ok(r)
    }

    pub fn request_mut(
        &mut self,
        tenant_id: &str,
        id: Uuid,
    ) -> CertManagerResult<&mut CertificateRequest> {
        let r = self
            .requests
            .get_mut(&id)
            .ok_or_else(|| CertManagerError::CertificateRequestNotFound(id.to_string()))?;
        check_tenant(&r.tenant_id, tenant_id)?;
        Ok(r)
    }

    pub fn list_requests_for(
        &self,
        tenant_id: &str,
        certificate_id: Uuid,
    ) -> Vec<&CertificateRequest> {
        self.requests
            .values()
            .filter(|r| r.tenant_id == tenant_id && r.certificate_id == certificate_id)
            .collect()
    }

    pub fn request_count(&self) -> usize {
        self.requests.len()
    }

    // ── Issuer ─────────────────────────────────────────────────────────

    pub fn put_issuer(&mut self, issuer: IssuerResource) -> Uuid {
        let id = issuer.id;
        self.issuers.insert(id, issuer);
        id
    }

    pub fn issuer_by_name(
        &self,
        tenant_id: &str,
        namespace: &str,
        name: &str,
    ) -> CertManagerResult<&IssuerResource> {
        self.issuers
            .values()
            .find(|i| i.tenant_id == tenant_id && i.namespace == namespace && i.name == name)
            .ok_or_else(|| {
                CertManagerError::IssuerNotFound(format!("{}/{}", namespace, name))
            })
    }

    pub fn issuer_count(&self) -> usize {
        self.issuers.len()
    }

    // ── ClusterIssuer ──────────────────────────────────────────────────

    pub fn put_cluster_issuer(&mut self, issuer: ClusterIssuer) -> Uuid {
        let id = issuer.id;
        self.cluster_issuers.insert(id, issuer);
        id
    }

    pub fn cluster_issuer_by_name(
        &self,
        tenant_id: &str,
        name: &str,
    ) -> CertManagerResult<&ClusterIssuer> {
        self.cluster_issuers
            .values()
            .find(|i| i.tenant_id == tenant_id && i.name == name)
            .ok_or_else(|| CertManagerError::ClusterIssuerNotFound(name.to_string()))
    }

    pub fn cluster_issuer_count(&self) -> usize {
        self.cluster_issuers.len()
    }
}

fn check_tenant(owner: &str, requester: &str) -> CertManagerResult<()> {
    if owner != requester {
        return Err(CertManagerError::CrossTenantDenied {
            owner_tenant: owner.to_string(),
            request_tenant: requester.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateRequestStatus, CertificateSpec, IssuerRef, IssuerRefKind, IssuerSpec,
        PrivateKeyPolicy, Usage,
    };
    use chrono::Utc;
    use std::collections::BTreeMap;

    fn mk_cert(tenant: &str) -> Certificate {
        Certificate {
            id: Uuid::new_v4(),
            name: "demo".into(),
            namespace: "default".into(),
            tenant_id: tenant.into(),
            spec: CertificateSpec {
                secret_name: "tls".into(),
                issuer_ref: IssuerRef {
                    name: "ca".into(),
                    kind: IssuerRefKind::ClusterIssuer,
                    group: "cert-manager.io".into(),
                },
                dns_names: vec!["x.example.com".into()],
                ip_addresses: vec![],
                uris: vec![],
                email_addresses: vec![],
                common_name: None,
                duration_seconds: 3600,
                renew_before_seconds: 600,
                usages: vec![],
                private_key: PrivateKeyPolicy::default(),
                is_ca: false,
                subject: None,
                secret_template_labels: BTreeMap::new(),
                secret_template_annotations: BTreeMap::new(),
            },
            status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        }
    }

    #[test]
    fn put_and_get_certificate_round_trip() {
        let mut s = CertManagerStore::new();
        let cert = mk_cert("t-1");
        let id = s.put_certificate(cert);
        assert!(s.certificate("t-1", id).is_ok());
    }

    #[test]
    fn cross_tenant_certificate_read_denied() {
        let mut s = CertManagerStore::new();
        let cert = mk_cert("t-1");
        let id = s.put_certificate(cert);
        let err = s.certificate("t-2", id).unwrap_err();
        assert!(matches!(err, CertManagerError::CrossTenantDenied { .. }));
    }

    #[test]
    fn missing_certificate_returns_not_found() {
        let s = CertManagerStore::new();
        let err = s.certificate("t-1", Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, CertManagerError::CertificateNotFound(_)));
    }

    #[test]
    fn list_certificates_scoped_to_tenant() {
        let mut s = CertManagerStore::new();
        s.put_certificate(mk_cert("t-1"));
        s.put_certificate(mk_cert("t-1"));
        s.put_certificate(mk_cert("t-2"));
        assert_eq!(s.list_certificates("t-1").len(), 2);
        assert_eq!(s.list_certificates("t-2").len(), 1);
        assert_eq!(s.certificate_count(), 3);
    }

    #[test]
    fn requests_filtered_by_certificate() {
        let mut s = CertManagerStore::new();
        let cert_id = s.put_certificate(mk_cert("t-1"));
        let req = CertificateRequest {
            id: Uuid::new_v4(),
            name: "demo-1".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            certificate_id: cert_id,
            revision: 1,
            issuer_ref: IssuerRef {
                name: "ca".into(),
                kind: IssuerRefKind::ClusterIssuer,
                group: "cert-manager.io".into(),
            },
            usages: vec![Usage::ServerAuth],
            dns_names: vec!["x".into()],
            ip_addresses: vec![],
            uris: vec![],
            email_addresses: vec![],
            common_name: None,
            duration_seconds: 3600,
            is_ca: false,
            created_at: Utc::now(),
            status: CertificateRequestStatus::default(),
        };
        s.put_request(req);
        assert_eq!(s.list_requests_for("t-1", cert_id).len(), 1);
        assert_eq!(s.list_requests_for("t-2", cert_id).len(), 0);
    }

    #[test]
    fn issuer_lookup_by_name_scoped_to_namespace() {
        let mut s = CertManagerStore::new();
        s.put_issuer(IssuerResource {
            id: Uuid::new_v4(),
            name: "ca".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            spec: IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
            created_at: Utc::now(),
        });
        assert!(s.issuer_by_name("t-1", "default", "ca").is_ok());
        assert!(s.issuer_by_name("t-1", "other", "ca").is_err());
    }

    #[test]
    fn cluster_issuer_lookup() {
        let mut s = CertManagerStore::new();
        s.put_cluster_issuer(ClusterIssuer {
            id: Uuid::new_v4(),
            name: "letsencrypt".into(),
            tenant_id: "t-1".into(),
            spec: IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
            created_at: Utc::now(),
        });
        assert!(s.cluster_issuer_by_name("t-1", "letsencrypt").is_ok());
        assert!(s.cluster_issuer_by_name("t-1", "missing").is_err());
        assert_eq!(s.cluster_issuer_count(), 1);
    }
}
