// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-certs — CertificateStore CRUD + lookup tests.
//!
//! Cite: cert-manager `pkg/controller/certificates/keymanager` +
//! `pkg/internal/apis/certmanager/validation/certificate.go` —
//! the controller writes issued certs into a per-tenant store keyed by
//! (tenant_id, secret_name) so issuance and renewal can find them.

use cave_certs::crds::{CertificateSpec, IssuerRef};
use cave_certs::models::{CertState, Certificate};
use cave_certs::store::CertificateStore;
use chrono::{Duration, Utc};
use uuid::Uuid;

const TENANT: &str = "tenant-store-test";

fn make_cert(domain: &str, secret_name: &str, state: CertState) -> Certificate {
    let now = Utc::now();
    Certificate {
        id: Uuid::new_v4(),
        domain: domain.into(),
        san_domains: vec![],
        issuer: "letsencrypt-prod".into(),
        not_before: now - Duration::days(1),
        not_after: now + Duration::days(89),
        serial_number: format!("01:{:02X}", secret_name.len()),
        fingerprint_sha256: format!("fp-{}", domain),
        state,
        auto_renew: true,
    }
}

/// Cite: cert-manager keymanager — store a cert by (tenant, secret_name) and
/// retrieve it back by the same key.
#[test]
fn store_and_retrieve_by_secret_name() {
    let mut store = CertificateStore::new();
    let cert = make_cert("api.example.com", "api-tls", CertState::Valid);
    store.put(TENANT, "api-tls", cert.clone());
    let found = store.get(TENANT, "api-tls").unwrap();
    assert_eq!(found.domain, "api.example.com");
}

/// Cite: cert-manager — a second `put` with the same key REPLACES the
/// existing cert (cert rotated).
#[test]
fn put_replaces_existing_entry() {
    let mut store = CertificateStore::new();
    let cert1 = make_cert("api.example.com", "api-tls", CertState::Valid);
    let cert2 = make_cert("api.example.com", "api-tls", CertState::Expiring);
    store.put(TENANT, "api-tls", cert1);
    store.put(TENANT, "api-tls", cert2);
    let found = store.get(TENANT, "api-tls").unwrap();
    assert_eq!(found.state, CertState::Expiring);
}

/// Cite: cave multi-tenant isolation — tenant A cannot read tenant B's cert.
#[test]
fn cross_tenant_lookup_returns_none() {
    let mut store = CertificateStore::new();
    let cert = make_cert("svc.b.com", "b-tls", CertState::Valid);
    store.put("tenant-b", "b-tls", cert);
    // Tenant A lookup must not return tenant B's cert.
    assert!(store.get("tenant-a", "b-tls").is_none());
}

/// Cite: cert-manager `remove` — called on certificate deletion.
#[test]
fn remove_clears_entry() {
    let mut store = CertificateStore::new();
    let cert = make_cert("svc.x.com", "x-tls", CertState::Valid);
    store.put(TENANT, "x-tls", cert);
    assert!(store.get(TENANT, "x-tls").is_some());
    store.remove(TENANT, "x-tls");
    assert!(store.get(TENANT, "x-tls").is_none());
}

/// Cite: cert-manager `list` — return all certs for a tenant, used by the
/// renewal controller to build the work queue.
#[test]
fn list_returns_all_certs_for_tenant() {
    let mut store = CertificateStore::new();
    store.put(
        TENANT,
        "a-tls",
        make_cert("a.example.com", "a-tls", CertState::Valid),
    );
    store.put(
        TENANT,
        "b-tls",
        make_cert("b.example.com", "b-tls", CertState::Expiring),
    );
    // Different tenant — must NOT appear.
    store.put(
        "other-tenant",
        "c-tls",
        make_cert("c.example.com", "c-tls", CertState::Valid),
    );

    let certs = store.list(TENANT);
    assert_eq!(certs.len(), 2);
    let domains: Vec<&str> = certs.iter().map(|c| c.domain.as_str()).collect();
    assert!(domains.contains(&"a.example.com"));
    assert!(domains.contains(&"b.example.com"));
}

/// Cite: cert-manager `expiring` helper — subset of `list` filtered to
/// certs within a renewal window (used by the trigger controller).
#[test]
fn list_expiring_filters_by_threshold_days() {
    let now = Utc::now();
    let mut store = CertificateStore::new();

    let mut expiring = make_cert("exp.example.com", "exp-tls", CertState::Expiring);
    expiring.not_after = now + Duration::days(10); // 10d remaining

    let mut fresh = make_cert("fresh.example.com", "fresh-tls", CertState::Valid);
    fresh.not_after = now + Duration::days(80); // 80d remaining

    store.put(TENANT, "exp-tls", expiring);
    store.put(TENANT, "fresh-tls", fresh);

    let expiring_list = store.list_expiring(TENANT, 30);
    assert_eq!(expiring_list.len(), 1);
    assert_eq!(expiring_list[0].domain, "exp.example.com");
}
