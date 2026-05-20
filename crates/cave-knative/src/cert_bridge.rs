// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cert-manager bridge — translate Knative Certificate CR to/from cert-manager.
//!
//! upstream: knative/serving — pkg/reconciler/certificate +
//! pkg/networking/v1alpha1/certificate_types.go
//!
//! cave-certs owns the cert-manager surface; this module is the
//! reconcile-shape that the Knative reconciler uses. Given a Knative
//! `Certificate` resource (host list + secret target), we produce the
//! cert-manager `Certificate` CR + the IssuerRef. The other direction
//! takes the cert-manager status and projects it back onto the Knative
//! Certificate's conditions.

use crate::meta::ObjectMeta;
use std::collections::HashMap;

#[derive(Default, Debug, Clone)]
pub struct KnativeCertificate {
    pub metadata: ObjectMeta,
    pub spec: KnativeCertificateSpec,
    pub status: KnativeCertificateStatus,
}

#[derive(Default, Debug, Clone)]
pub struct KnativeCertificateSpec {
    pub dns_names: Vec<String>,
    pub secret_name: String,
    pub issuer_ref: IssuerRef,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct IssuerRef {
    pub name: String,
    pub kind: String,  // "ClusterIssuer" | "Issuer"
    pub group: String, // typically "cert-manager.io"
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct KnativeCertificateStatus {
    pub ready: bool,
    pub not_after: Option<u64>,
    pub conditions: HashMap<String, String>,
}

/// cert-manager `Certificate` projection (a faithful shape of the
/// upstream CRD; the bridge fills it from a `KnativeCertificate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertManagerCertificate {
    pub name: String,
    pub namespace: String,
    pub dns_names: Vec<String>,
    pub secret_name: String,
    pub issuer_ref: IssuerRef,
    pub usages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertManagerStatus {
    pub ready: bool,
    pub not_after_unix_seconds: Option<u64>,
    pub failure_reason: Option<String>,
}

impl KnativeCertificate {
    pub fn new(tenant_id: &str, name: &str, secret: &str) -> Self {
        let mut c = KnativeCertificate::default();
        c.metadata = ObjectMeta::with_creator(tenant_id);
        c.metadata.name = name.to_string();
        c.spec.secret_name = secret.to_string();
        c
    }
}

/// Build the cert-manager Certificate CR for a Knative one.
pub fn to_cert_manager(knative: &KnativeCertificate) -> Result<CertManagerCertificate, String> {
    if knative.spec.dns_names.is_empty() {
        return Err("knative Certificate must list at least one DNS name".into());
    }
    if knative.spec.secret_name.is_empty() {
        return Err("knative Certificate must set secret_name".into());
    }
    if knative.spec.issuer_ref.name.is_empty() {
        return Err("issuer_ref.name must be set".into());
    }
    let issuer = IssuerRef {
        name: knative.spec.issuer_ref.name.clone(),
        kind: if knative.spec.issuer_ref.kind.is_empty() {
            "ClusterIssuer".into()
        } else {
            knative.spec.issuer_ref.kind.clone()
        },
        group: if knative.spec.issuer_ref.group.is_empty() {
            "cert-manager.io".into()
        } else {
            knative.spec.issuer_ref.group.clone()
        },
    };
    Ok(CertManagerCertificate {
        name: format!("knative-{}", knative.metadata.name),
        namespace: knative.metadata.namespace.clone(),
        dns_names: knative.spec.dns_names.clone(),
        secret_name: knative.spec.secret_name.clone(),
        issuer_ref: issuer,
        usages: vec![
            "server auth".into(),
            "digital signature".into(),
            "key encipherment".into(),
        ],
    })
}

/// Project cert-manager status back onto the Knative Certificate
/// conditions.  Returns the new conditions block.
pub fn project_status_back(cm: &CertManagerStatus) -> KnativeCertificateStatus {
    let mut conditions = HashMap::new();
    conditions.insert(
        "Ready".into(),
        if cm.ready {
            "True".to_string()
        } else {
            "False".to_string()
        },
    );
    if let Some(reason) = &cm.failure_reason {
        conditions.insert("Failed".into(), reason.clone());
    }
    KnativeCertificateStatus {
        ready: cm.ready,
        not_after: cm.not_after_unix_seconds,
        conditions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cert(name: &str) -> KnativeCertificate {
        let mut c = KnativeCertificate::new("t", name, "tls-secret");
        c.spec.dns_names = vec!["svc.example.com".into()];
        c.spec.issuer_ref = IssuerRef {
            name: "letsencrypt-prod".into(),
            kind: "ClusterIssuer".into(),
            group: "cert-manager.io".into(),
        };
        c
    }

    #[test]
    fn to_cert_manager_pins_default_usages() {
        let cm = to_cert_manager(&cert("c1")).unwrap();
        assert!(cm.usages.contains(&"server auth".to_string()));
        assert!(cm.usages.contains(&"digital signature".to_string()));
        assert!(cm.usages.contains(&"key encipherment".to_string()));
    }

    #[test]
    fn to_cert_manager_prefixes_name() {
        let cm = to_cert_manager(&cert("svc-tls")).unwrap();
        assert_eq!(cm.name, "knative-svc-tls");
    }

    #[test]
    fn to_cert_manager_carries_dns_names() {
        let mut c = cert("c");
        c.spec.dns_names = vec!["a.com".into(), "b.com".into()];
        let cm = to_cert_manager(&c).unwrap();
        assert_eq!(cm.dns_names, vec!["a.com".to_string(), "b.com".to_string()]);
    }

    #[test]
    fn to_cert_manager_rejects_missing_dns() {
        let mut c = cert("c");
        c.spec.dns_names.clear();
        assert!(to_cert_manager(&c).is_err());
    }

    #[test]
    fn to_cert_manager_rejects_empty_issuer_name() {
        let mut c = cert("c");
        c.spec.issuer_ref.name.clear();
        assert!(to_cert_manager(&c).is_err());
    }

    #[test]
    fn to_cert_manager_defaults_kind_to_cluster_issuer() {
        let mut c = cert("c");
        c.spec.issuer_ref.kind.clear();
        let cm = to_cert_manager(&c).unwrap();
        assert_eq!(cm.issuer_ref.kind, "ClusterIssuer");
    }

    #[test]
    fn project_status_ready_marks_ready_condition_true() {
        let cm = CertManagerStatus {
            ready: true,
            not_after_unix_seconds: Some(1_700_000_000),
            failure_reason: None,
        };
        let s = project_status_back(&cm);
        assert!(s.ready);
        assert_eq!(s.conditions.get("Ready").map(String::as_str), Some("True"));
        assert!(s.conditions.get("Failed").is_none());
    }

    #[test]
    fn project_status_failure_records_failed_condition() {
        let cm = CertManagerStatus {
            ready: false,
            not_after_unix_seconds: None,
            failure_reason: Some("DNS01-challenge failed".into()),
        };
        let s = project_status_back(&cm);
        assert!(!s.ready);
        assert_eq!(s.conditions.get("Ready").map(String::as_str), Some("False"));
        assert_eq!(
            s.conditions.get("Failed").map(String::as_str),
            Some("DNS01-challenge failed")
        );
    }
}
