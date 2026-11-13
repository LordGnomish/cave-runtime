// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-memory certificate store — keyed by `(tenant_id, secret_name)`.
//!
//! Cite: cert-manager `pkg/controller/certificates/keymanager` —
//! issued certificates are stored in Kubernetes Secrets (tls.crt + tls.key).
//! cave's store is an in-process map that mirrors that contract without
//! requiring a K8s API server. The key is `(tenant_id, secret_name)` which
//! matches `CertificateSpec.secret_name`.

use crate::models::Certificate;
use chrono::Utc;
use std::collections::HashMap;

/// Cite: cert-manager keymanager + cert controller — single store shared by
/// the issuance pipeline. In production this would be backed by the
/// Kubernetes Secret API.
#[derive(Debug, Default)]
pub struct CertificateStore {
    /// Nested map: `tenant_id → secret_name → Certificate`.
    entries: HashMap<String, HashMap<String, Certificate>>,
}

impl CertificateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the certificate at `(tenant_id, secret_name)`.
    pub fn put(&mut self, tenant_id: &str, secret_name: &str, cert: Certificate) {
        self.entries
            .entry(tenant_id.into())
            .or_default()
            .insert(secret_name.into(), cert);
    }

    /// Look up a certificate by `(tenant_id, secret_name)`.
    /// Returns `None` if no cert is stored for that key, or if the
    /// tenant does not exist (cross-tenant isolation).
    pub fn get(&self, tenant_id: &str, secret_name: &str) -> Option<&Certificate> {
        self.entries.get(tenant_id)?.get(secret_name)
    }

    /// Remove the certificate at `(tenant_id, secret_name)`.
    /// Returns `true` when an entry was removed, `false` when it was absent.
    pub fn remove(&mut self, tenant_id: &str, secret_name: &str) -> bool {
        self.entries
            .get_mut(tenant_id)
            .map(|m| m.remove(secret_name).is_some())
            .unwrap_or(false)
    }

    /// Return all certificates for `tenant_id` (empty vec if tenant has none).
    /// Cite: cert-manager renewal controller builds its work queue this way.
    pub fn list(&self, tenant_id: &str) -> Vec<&Certificate> {
        self.entries
            .get(tenant_id)
            .map(|m| m.values().collect())
            .unwrap_or_default()
    }

    /// Return certificates for `tenant_id` whose `not_after` is within
    /// `days` days but not yet expired.
    ///
    /// Cite: cert-manager trigger controller `shouldReissue` — build the
    /// list of near-expiry certs to enqueue for renewal.
    pub fn list_expiring(&self, tenant_id: &str, days: i64) -> Vec<&Certificate> {
        let now = Utc::now();
        let threshold = now + chrono::Duration::days(days);
        self.list(tenant_id)
            .into_iter()
            .filter(|c| c.not_after > now && c.not_after <= threshold)
            .collect()
    }

    /// Total number of certificates across all tenants.
    pub fn total_count(&self) -> usize {
        self.entries.values().map(|m| m.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CertState, Certificate};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn cert(domain: &str, days_remaining: i64) -> Certificate {
        let now = Utc::now();
        Certificate {
            id: Uuid::new_v4(),
            domain: domain.into(),
            san_domains: vec![],
            issuer: "test-ca".into(),
            not_before: now - Duration::days(1),
            not_after: now + Duration::days(days_remaining),
            serial_number: "01:00".into(),
            fingerprint_sha256: format!("fp-{}", domain),
            state: CertState::Valid,
            auto_renew: true,
        }
    }

    #[test]
    fn put_and_get_round_trip() {
        let mut store = CertificateStore::new();
        store.put("t1", "api-tls", cert("api.example.com", 90));
        let c = store.get("t1", "api-tls").unwrap();
        assert_eq!(c.domain, "api.example.com");
    }

    #[test]
    fn get_cross_tenant_returns_none() {
        let mut store = CertificateStore::new();
        store.put("t1", "api-tls", cert("api.example.com", 90));
        assert!(store.get("t2", "api-tls").is_none());
    }

    #[test]
    fn remove_returns_false_when_absent() {
        let mut store = CertificateStore::new();
        assert!(!store.remove("t1", "missing"));
    }

    #[test]
    fn list_expiring_excludes_fresh_certs() {
        let mut store = CertificateStore::new();
        store.put("t1", "a", cert("a.example.com", 10));
        store.put("t1", "b", cert("b.example.com", 90));
        let exp = store.list_expiring("t1", 30);
        assert_eq!(exp.len(), 1);
        assert_eq!(exp[0].domain, "a.example.com");
    }

    #[test]
    fn total_count_spans_tenants() {
        let mut store = CertificateStore::new();
        store.put("t1", "a", cert("a.example.com", 90));
        store.put("t2", "b", cert("b.example.com", 90));
        assert_eq!(store.total_count(), 2);
    }
}
