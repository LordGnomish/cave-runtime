// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Reconcile loop — the certificate controller.
//!
//! Cite: `pkg/controller/certificates/issuing/issuing_controller.go::reconcile`
//! — given a Certificate, cert-manager:
//!   1. picks the next CertificateRequest revision
//!   2. resolves the IssuerRef → Issuer / ClusterIssuer
//!   3. dispatches to the issuer backend
//!   4. writes the Secret + sets `Ready=True` on the Certificate
//!
//! cave-cert-manager keeps the same five-step shape with deterministic
//! tenant scoping all the way down.

use crate::error::CertManagerResult;
use crate::issuer::{IssueOutcome, IssuerRegistry};
use crate::models::{
    Certificate, CertificateCondition, CertificateConditionType, CertificateRequest,
    CertificateRequestCondition, CertificateRequestConditionType, CertificateRequestStatus,
    CertificateStatus, ConditionStatus, IssuerRefKind, IssuerSpec, Usage,
};
use crate::renewal::RenewalScheduler;
use crate::secret::{SecretMaterializer, SecretRecord};
use crate::store::CertManagerStore;
use chrono::Utc;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Event surface emitted by the reconciler so the cave-runtime event
/// bus / cave-oncall pipeline can react.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconcileEvent {
    Issued { certificate_id: Uuid, serial: String },
    Renewed { certificate_id: Uuid, serial: String },
    Failed { certificate_id: Uuid, message: String },
}

/// Outcome of a single reconcile step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileResult {
    pub request_id: Uuid,
    pub secret: SecretRecord,
    pub events: Vec<ReconcileEvent>,
    pub previous_revision: u64,
    pub new_revision: u64,
}

pub struct CertificateController<'a> {
    pub store: &'a mut CertManagerStore,
    pub registry: &'a mut IssuerRegistry,
    pub secrets: &'a mut SecretMaterializer,
}

impl<'a> CertificateController<'a> {
    /// Drive one Certificate through `project → dispatch → materialise`.
    pub fn reconcile(
        &mut self,
        tenant_id: &str,
        certificate_id: Uuid,
    ) -> CertManagerResult<ReconcileResult> {
        let spec = {
            let cert = self.store.certificate(tenant_id, certificate_id)?;
            cert.spec.clone()
        };
        spec.validate()?;

        // 1. Pick next revision = max(existing) + 1.
        let previous_revision = self
            .store
            .list_requests_for(tenant_id, certificate_id)
            .iter()
            .map(|r| r.revision)
            .max()
            .unwrap_or(0);
        let new_revision = previous_revision + 1;

        let (cert_name, cert_namespace, _cert_tenant) = {
            let cert = self.store.certificate(tenant_id, certificate_id)?;
            (cert.name.clone(), cert.namespace.clone(), cert.tenant_id.clone())
        };

        // 2. Resolve IssuerRef.
        let issuer_spec = self.resolve_issuer(tenant_id, &cert_namespace, &spec.issuer_ref)?;

        // 3. Project the Certificate into a CertificateRequest.
        let req = CertificateRequest {
            id: Uuid::new_v4(),
            name: format!("{}-{}", cert_name, new_revision),
            namespace: cert_namespace.clone(),
            tenant_id: tenant_id.to_string(),
            certificate_id,
            revision: new_revision,
            issuer_ref: spec.issuer_ref.clone(),
            usages: if spec.usages.is_empty() {
                vec![Usage::ServerAuth, Usage::DigitalSignature]
            } else {
                spec.usages.clone()
            },
            dns_names: spec.dns_names.clone(),
            ip_addresses: spec.ip_addresses.clone(),
            uris: spec.uris.clone(),
            email_addresses: spec.email_addresses.clone(),
            common_name: spec.common_name.clone(),
            duration_seconds: spec.duration_seconds,
            is_ca: spec.is_ca,
            created_at: Utc::now(),
            status: CertificateRequestStatus::default(),
        };
        let req_id = self.store.put_request(req.clone());

        // 4. Dispatch to issuer backend.
        let outcome = match self.registry.issue(&issuer_spec, &req) {
            Ok(o) => o,
            Err(e) => {
                // Stamp Failed on both the request and the Certificate
                // status — observers should see exactly one source of
                // truth.
                let msg = e.to_string();
                self.mark_request_invalid(tenant_id, req_id, &msg)?;
                self.mark_cert_failed(tenant_id, certificate_id, &msg)?;
                return Err(e);
            }
        };

        // 5. Materialise the Secret.
        let cert_snapshot = self.store.certificate(tenant_id, certificate_id)?.clone();
        let rec = self.secrets.materialise(&cert_snapshot, &outcome)?;

        // 6. Stamp success on request + certificate.
        self.mark_request_ready(tenant_id, req_id, &outcome)?;
        self.mark_cert_ready(tenant_id, certificate_id, &outcome, new_revision, &rec)?;

        let event = if previous_revision == 0 {
            ReconcileEvent::Issued {
                certificate_id,
                serial: outcome.serial.clone(),
            }
        } else {
            ReconcileEvent::Renewed {
                certificate_id,
                serial: outcome.serial.clone(),
            }
        };

        Ok(ReconcileResult {
            request_id: req_id,
            secret: rec,
            events: vec![event],
            previous_revision,
            new_revision,
        })
    }

    /// Drain the renewal scheduler, reconciling every due Certificate.
    /// Returns the resolved reconciliation results in the order they
    /// were drained.
    pub fn reconcile_due(
        &mut self,
        tenant_id: &str,
        now: chrono::DateTime<Utc>,
    ) -> Vec<CertManagerResult<ReconcileResult>> {
        let certs: Vec<Certificate> = self
            .store
            .list_certificates(tenant_id)
            .iter()
            .map(|c| (*c).clone())
            .collect();
        let plans = RenewalScheduler.plan(&certs, now);
        plans
            .into_iter()
            .map(|p| self.reconcile(tenant_id, p.certificate_id))
            .collect()
    }

    fn resolve_issuer(
        &self,
        tenant_id: &str,
        namespace: &str,
        ref_: &crate::models::IssuerRef,
    ) -> CertManagerResult<IssuerSpec> {
        match ref_.kind {
            IssuerRefKind::Issuer => Ok(self
                .store
                .issuer_by_name(tenant_id, namespace, &ref_.name)?
                .spec
                .clone()),
            IssuerRefKind::ClusterIssuer => Ok(self
                .store
                .cluster_issuer_by_name(tenant_id, &ref_.name)?
                .spec
                .clone()),
        }
    }

    fn mark_request_ready(
        &mut self,
        tenant_id: &str,
        req_id: Uuid,
        outcome: &IssueOutcome,
    ) -> CertManagerResult<()> {
        let req = self.store.request_mut(tenant_id, req_id)?;
        req.status = CertificateRequestStatus {
            conditions: vec![CertificateRequestCondition {
                kind: CertificateRequestConditionType::Ready,
                status: ConditionStatus::True,
                reason: Some("Issued".into()),
                message: None,
                last_transition_time: Utc::now(),
            }],
            certificate_chain_pem: Some(outcome.certificate_chain_pem.clone()),
            ca_pem: Some(outcome.ca_pem.clone()),
            failure_time: None,
        };
        Ok(())
    }

    fn mark_request_invalid(
        &mut self,
        tenant_id: &str,
        req_id: Uuid,
        message: &str,
    ) -> CertManagerResult<()> {
        let req = self.store.request_mut(tenant_id, req_id)?;
        req.status = CertificateRequestStatus {
            conditions: vec![CertificateRequestCondition {
                kind: CertificateRequestConditionType::InvalidRequest,
                status: ConditionStatus::True,
                reason: Some("IssuerError".into()),
                message: Some(message.to_string()),
                last_transition_time: Utc::now(),
            }],
            certificate_chain_pem: None,
            ca_pem: None,
            failure_time: Some(Utc::now()),
        };
        Ok(())
    }

    fn mark_cert_ready(
        &mut self,
        tenant_id: &str,
        cert_id: Uuid,
        outcome: &IssueOutcome,
        revision: u64,
        rec: &SecretRecord,
    ) -> CertManagerResult<()> {
        let cert = self.store.certificate_mut(tenant_id, cert_id)?;
        let renew_before = chrono::Duration::seconds(cert.spec.renew_before_seconds);
        cert.status = Some(CertificateStatus {
            conditions: vec![CertificateCondition {
                kind: CertificateConditionType::Ready,
                status: ConditionStatus::True,
                reason: Some("Ready".into()),
                message: None,
                last_transition_time: Utc::now(),
            }],
            serial: Some(outcome.serial.clone()),
            not_before: Some(outcome.not_before),
            not_after: Some(outcome.not_after),
            renewal_time: Some(outcome.not_after - renew_before),
            revision,
            last_failure_message: None,
            secret_ref: Some(SecretMaterializer::secret_ref(rec)),
        });
        cert.updated_at = Utc::now();
        Ok(())
    }

    fn mark_cert_failed(
        &mut self,
        tenant_id: &str,
        cert_id: Uuid,
        message: &str,
    ) -> CertManagerResult<()> {
        let cert = self.store.certificate_mut(tenant_id, cert_id)?;
        let conditions = vec![CertificateCondition {
            kind: CertificateConditionType::Ready,
            status: ConditionStatus::False,
            reason: Some("IssuerError".into()),
            message: Some(message.to_string()),
            last_transition_time: Utc::now(),
        }];
        let new_status = match cert.status.take() {
            Some(mut s) => {
                s.conditions = conditions;
                s.last_failure_message = Some(message.to_string());
                s
            }
            None => CertificateStatus {
                conditions,
                serial: None,
                not_before: None,
                not_after: None,
                renewal_time: None,
                revision: 0,
                last_failure_message: Some(message.to_string()),
                secret_ref: None,
            },
        };
        cert.status = Some(new_status);
        Ok(())
    }
}

/// Constructor that bundles a default store + registry + materializer for
/// integration tests / smoke runs.
#[derive(Default)]
pub struct CertControlPlane {
    pub store: CertManagerStore,
    pub registry: IssuerRegistry,
    pub secrets: SecretMaterializer,
}

impl CertControlPlane {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn controller(&mut self) -> CertificateController<'_> {
        CertificateController {
            store: &mut self.store,
            registry: &mut self.registry,
            secrets: &mut self.secrets,
        }
    }
}

/// Helper for callers that want default labels stamped onto certs.
pub fn default_labels() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    m.insert(
        "app.kubernetes.io/managed-by".into(),
        "cave-cert-manager".into(),
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CertManagerError;
    use crate::models::{
        CertificateSpec, ClusterIssuer, IssuerRef, IssuerRefKind, IssuerSpec, PrivateKeyPolicy,
    };
    use std::collections::BTreeMap;

    fn cluster_selfsigned(tenant: &str, name: &str) -> ClusterIssuer {
        ClusterIssuer {
            id: Uuid::new_v4(),
            name: name.into(),
            tenant_id: tenant.into(),
            spec: IssuerSpec::SelfSigned {
                crl_distribution_points: vec![],
            },
            created_at: Utc::now(),
        }
    }

    fn cert(tenant: &str, name: &str, issuer: &str) -> Certificate {
        Certificate {
            id: Uuid::new_v4(),
            name: name.into(),
            namespace: "default".into(),
            tenant_id: tenant.into(),
            spec: CertificateSpec {
                secret_name: format!("{}-tls", name),
                issuer_ref: IssuerRef {
                    name: issuer.into(),
                    kind: IssuerRefKind::ClusterIssuer,
                    group: "cert-manager.io".into(),
                },
                dns_names: vec![format!("{}.example.com", name)],
                ip_addresses: vec![],
                uris: vec![],
                email_addresses: vec![],
                common_name: None,
                duration_seconds: 90 * 24 * 3600,
                renew_before_seconds: 30 * 24 * 3600,
                usages: vec![Usage::ServerAuth],
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
    fn end_to_end_selfsigned_yields_ready_certificate() {
        let mut cp = CertControlPlane::new();
        cp.store.put_cluster_issuer(cluster_selfsigned("t-1", "selfsigned"));
        let id = cp.store.put_certificate(cert("t-1", "demo", "selfsigned"));
        let res = cp.controller().reconcile("t-1", id).unwrap();
        assert_eq!(res.new_revision, 1);
        assert_eq!(res.events.len(), 1);
        assert!(matches!(res.events[0], ReconcileEvent::Issued { .. }));
        let cert = cp.store.certificate("t-1", id).unwrap();
        let status = cert.status.as_ref().unwrap();
        assert_eq!(status.revision, 1);
        assert_eq!(
            status.conditions[0].kind,
            CertificateConditionType::Ready
        );
        assert_eq!(status.conditions[0].status, ConditionStatus::True);
    }

    #[test]
    fn reissue_increments_revision() {
        let mut cp = CertControlPlane::new();
        cp.store.put_cluster_issuer(cluster_selfsigned("t-1", "selfsigned"));
        let id = cp.store.put_certificate(cert("t-1", "demo", "selfsigned"));
        let first = cp.controller().reconcile("t-1", id).unwrap();
        let second = cp.controller().reconcile("t-1", id).unwrap();
        assert_eq!(first.new_revision, 1);
        assert_eq!(second.new_revision, 2);
        assert!(matches!(second.events[0], ReconcileEvent::Renewed { .. }));
    }

    #[test]
    fn missing_issuer_marks_certificate_failed() {
        let mut cp = CertControlPlane::new();
        let id = cp.store.put_certificate(cert("t-1", "demo", "missing"));
        let err = cp.controller().reconcile("t-1", id).unwrap_err();
        assert!(matches!(err, CertManagerError::ClusterIssuerNotFound(_)));
        // Resolution errors short-circuit before request creation, so the
        // certificate status is untouched. Verify status is still None.
        let cert_ro = cp.store.certificate("t-1", id).unwrap();
        assert!(cert_ro.status.is_none());
    }

    #[test]
    fn invalid_spec_fails_validation() {
        let mut cp = CertControlPlane::new();
        cp.store.put_cluster_issuer(cluster_selfsigned("t-1", "selfsigned"));
        let mut c = cert("t-1", "demo", "selfsigned");
        c.spec.dns_names.clear();
        let id = cp.store.put_certificate(c);
        let err = cp.controller().reconcile("t-1", id).unwrap_err();
        assert!(matches!(err, CertManagerError::EmptyDnsNames));
    }

    #[test]
    fn cross_tenant_reconcile_denied() {
        let mut cp = CertControlPlane::new();
        cp.store.put_cluster_issuer(cluster_selfsigned("t-1", "selfsigned"));
        let id = cp.store.put_certificate(cert("t-1", "demo", "selfsigned"));
        let err = cp.controller().reconcile("t-2", id).unwrap_err();
        assert!(matches!(err, CertManagerError::CrossTenantDenied { .. }));
    }

    #[test]
    fn reconcile_due_processes_initial_issuance() {
        let mut cp = CertControlPlane::new();
        cp.store.put_cluster_issuer(cluster_selfsigned("t-1", "selfsigned"));
        cp.store.put_certificate(cert("t-1", "alpha", "selfsigned"));
        cp.store.put_certificate(cert("t-1", "beta", "selfsigned"));
        let results = cp.controller().reconcile_due("t-1", Utc::now());
        assert_eq!(results.len(), 2);
        for r in &results {
            r.as_ref().unwrap();
        }
        assert_eq!(cp.secrets.len(), 2);
    }

    #[test]
    fn default_labels_include_managed_by() {
        assert_eq!(
            default_labels().get("app.kubernetes.io/managed-by"),
            Some(&"cave-cert-manager".to_string())
        );
    }
}
