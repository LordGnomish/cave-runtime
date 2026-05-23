// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Secret reconciler — materialises a successful [`IssueOutcome`] into
//! a Kubernetes-shaped Secret record.
//!
//! Cite: `pkg/controller/certificates/issuing/issuing_controller.go::reconcile`
//! — cert-manager writes the `tls.crt`, `tls.key`, and `ca.crt` keys
//! into the `kubernetes.io/tls` Secret named by `Certificate.spec.secretName`.
//!
//! The Secret store is opaque — cave-runtime mounts the real reconciler
//! against cave-store / the Kubernetes API; cave-cert-manager owns the
//! shape only.

use crate::error::{CertManagerError, CertManagerResult};
use crate::issuer::IssueOutcome;
use crate::models::{Certificate, SecretRef};
use chrono::{DateTime, Utc};
use std::collections::{BTreeMap, HashMap};

/// `kubernetes.io/tls` Secret stand-in. Carries the data keys + the
/// labels and annotations propagated from
/// [`CertificateSpec::secret_template_labels`] /
/// [`CertificateSpec::secret_template_annotations`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRecord {
    pub name: String,
    pub namespace: String,
    pub tenant_id: String,
    pub r#type: String,
    pub data: BTreeMap<String, Vec<u8>>,
    pub labels: BTreeMap<String, String>,
    pub annotations: BTreeMap<String, String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct SecretMaterializer {
    by_ref: HashMap<(String, String, String), SecretRecord>,
}

impl SecretMaterializer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Materialise `outcome` into the Secret named by `cert.spec.secret_name`.
    /// Returns the persisted record.
    pub fn materialise(
        &mut self,
        cert: &Certificate,
        outcome: &IssueOutcome,
    ) -> CertManagerResult<SecretRecord> {
        let mut data = BTreeMap::new();
        // cert-manager always splits the chain into `tls.crt` (full chain
        // with leaf first) + `ca.crt` (issuer bundle).
        data.insert(
            "tls.crt".into(),
            outcome.certificate_chain_pem.clone().into_bytes(),
        );
        data.insert("ca.crt".into(), outcome.ca_pem.clone().into_bytes());
        // tls.key — the private key handle reference, NOT the key bytes.
        // cave-cert-manager NEVER stores raw key material in process
        // memory. The handle resolves through cave-vault at TLS-handshake
        // time. Cite: cert-manager `pkg/controller/certificates/keymanager/keymanager.go`
        // for the analogous design (tls.key bytes), where we instead
        // emit a `keychain:` reference and let cave-net deref it.
        let handle = format!("keychain:cave-cert-{}-{}", cert.namespace, cert.spec.secret_name);
        data.insert("tls.key".into(), handle.into_bytes());

        let labels = cert.spec.secret_template_labels.clone();
        let mut annotations = cert.spec.secret_template_annotations.clone();
        annotations.insert(
            "cert-manager.io/certificate-name".into(),
            cert.name.clone(),
        );
        annotations.insert("cert-manager.io/serial".into(), outcome.serial.clone());

        let rec = SecretRecord {
            name: cert.spec.secret_name.clone(),
            namespace: cert.namespace.clone(),
            tenant_id: cert.tenant_id.clone(),
            r#type: "kubernetes.io/tls".into(),
            data,
            labels,
            annotations,
            updated_at: Utc::now(),
        };
        self.by_ref.insert(self.key_for(&rec), rec.clone());
        Ok(rec)
    }

    pub fn get(
        &self,
        tenant_id: &str,
        namespace: &str,
        name: &str,
    ) -> CertManagerResult<&SecretRecord> {
        self.by_ref
            .get(&(tenant_id.to_string(), namespace.to_string(), name.to_string()))
            .ok_or_else(|| {
                CertManagerError::SecretNotFound(format!("{}/{}", namespace, name))
            })
    }

    pub fn secret_ref(rec: &SecretRecord) -> SecretRef {
        SecretRef {
            name: rec.name.clone(),
            namespace: rec.namespace.clone(),
        }
    }

    pub fn len(&self) -> usize {
        self.by_ref.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_ref.is_empty()
    }

    fn key_for(&self, rec: &SecretRecord) -> (String, String, String) {
        (rec.tenant_id.clone(), rec.namespace.clone(), rec.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CertificateSpec, IssuerRef, IssuerRefKind, PrivateKeyPolicy,
    };
    use chrono::Duration;
    use uuid::Uuid;

    fn cert_with_template(
        labels: BTreeMap<String, String>,
        annotations: BTreeMap<String, String>,
    ) -> Certificate {
        Certificate {
            id: Uuid::new_v4(),
            name: "demo".into(),
            namespace: "default".into(),
            tenant_id: "t-1".into(),
            spec: CertificateSpec {
                secret_name: "tls".into(),
                issuer_ref: IssuerRef {
                    name: "ca".into(),
                    kind: IssuerRefKind::ClusterIssuer,
                    group: "cert-manager.io".into(),
                },
                dns_names: vec!["example.com".into()],
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
                secret_template_labels: labels,
                secret_template_annotations: annotations,
            },
            status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
        }
    }

    fn outcome() -> IssueOutcome {
        IssueOutcome {
            certificate_chain_pem: "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into(),
            ca_pem: "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n".into(),
            not_before: Utc::now(),
            not_after: Utc::now() + Duration::seconds(3600),
            serial: "deadbeef".into(),
        }
    }

    #[test]
    fn produces_tls_secret_shape() {
        let mut m = SecretMaterializer::new();
        let rec = m
            .materialise(
                &cert_with_template(BTreeMap::new(), BTreeMap::new()),
                &outcome(),
            )
            .unwrap();
        assert_eq!(rec.r#type, "kubernetes.io/tls");
        assert!(rec.data.contains_key("tls.crt"));
        assert!(rec.data.contains_key("ca.crt"));
        assert!(rec.data.contains_key("tls.key"));
    }

    #[test]
    fn tls_key_is_a_keychain_handle_not_raw_bytes() {
        let mut m = SecretMaterializer::new();
        let rec = m
            .materialise(
                &cert_with_template(BTreeMap::new(), BTreeMap::new()),
                &outcome(),
            )
            .unwrap();
        let key_bytes = rec.data.get("tls.key").unwrap();
        let s = std::str::from_utf8(key_bytes).unwrap();
        assert!(s.starts_with("keychain:"));
        assert!(!s.contains("PRIVATE KEY"));
    }

    #[test]
    fn secret_template_labels_propagated() {
        let mut labels = BTreeMap::new();
        labels.insert("app".into(), "ingress".into());
        let mut m = SecretMaterializer::new();
        let rec = m
            .materialise(&cert_with_template(labels.clone(), BTreeMap::new()), &outcome())
            .unwrap();
        assert_eq!(rec.labels, labels);
    }

    #[test]
    fn serial_lands_in_annotations() {
        let mut m = SecretMaterializer::new();
        let rec = m
            .materialise(&cert_with_template(BTreeMap::new(), BTreeMap::new()), &outcome())
            .unwrap();
        assert_eq!(
            rec.annotations.get("cert-manager.io/serial").map(String::as_str),
            Some("deadbeef")
        );
    }

    #[test]
    fn get_returns_stored_record() {
        let mut m = SecretMaterializer::new();
        let cert = cert_with_template(BTreeMap::new(), BTreeMap::new());
        let _ = m.materialise(&cert, &outcome()).unwrap();
        let r = m.get("t-1", "default", "tls").unwrap();
        assert_eq!(r.r#type, "kubernetes.io/tls");
    }

    #[test]
    fn get_unknown_returns_secret_not_found() {
        let m = SecretMaterializer::new();
        let err = m.get("t-1", "default", "nope").unwrap_err();
        assert!(matches!(err, CertManagerError::SecretNotFound(_)));
    }

    #[test]
    fn secret_ref_extracts_namespace_and_name() {
        let mut m = SecretMaterializer::new();
        let rec = m
            .materialise(&cert_with_template(BTreeMap::new(), BTreeMap::new()), &outcome())
            .unwrap();
        let r = SecretMaterializer::secret_ref(&rec);
        assert_eq!(r.name, "tls");
        assert_eq!(r.namespace, "default");
    }
}
