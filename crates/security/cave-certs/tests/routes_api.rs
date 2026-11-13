// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-certs — REST routes API tests.
//!
//! Cite: cert-manager exposes its operations via the K8s CRD API
//! (certificates.cert-manager.io, certificaterequests.cert-manager.io).
//! cave's HTTP API mirrors the key operations as a REST façade.

use cave_certs::routes_api::{
    CertListResponse, CertSummary, SelfSignedIssueRequest, SelfSignedIssueResponse,
    handle_issue_selfsigned, handle_list_certs, handle_remove_cert,
};

const TENANT: &str = "tenant-routes-test";

/// Cite: cert-manager newOrder → cert issuance flow — POST /api/certs/issue
/// creates a self-signed cert and returns the PEM.
#[test]
fn issue_selfsigned_returns_certificate_pem() {
    let req = SelfSignedIssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec!["api.routes-test.example.com".into()],
        common_name: Some("api.routes-test.example.com".into()),
        secret_name: "api-tls".into(),
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    let resp = handle_issue_selfsigned(req).unwrap();
    assert!(resp.certificate_pem.contains("CERTIFICATE"));
    assert!(!resp.secret_name.is_empty());
}

/// Cite: cert-manager — GET /api/certs returns all certs for the tenant.
#[test]
fn list_certs_returns_previously_issued() {
    let req1 = SelfSignedIssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec!["svc-a.routes-test.example.com".into()],
        common_name: None,
        secret_name: "svc-a-tls".into(),
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    let req2 = SelfSignedIssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec!["svc-b.routes-test.example.com".into()],
        common_name: None,
        secret_name: "svc-b-tls".into(),
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    handle_issue_selfsigned(req1).unwrap();
    handle_issue_selfsigned(req2).unwrap();

    // Note: because our store is global + test-scoped there may be other
    // certs from parallel tests — just check ≥2 for this tenant.
    let list = handle_list_certs(TENANT);
    assert!(
        list.certs.len() >= 2,
        "Expected at least 2 certs, got {}",
        list.certs.len()
    );
}

/// Cite: cert-manager — DELETE /api/certs/:secret_name removes the cert.
#[test]
fn remove_cert_clears_it_from_list() {
    let tenant = "tenant-routes-remove";
    let req = SelfSignedIssueRequest {
        tenant_id: tenant.into(),
        dns_names: vec!["svc.remove-test.example.com".into()],
        common_name: None,
        secret_name: "remove-tls".into(),
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    handle_issue_selfsigned(req).unwrap();
    let before = handle_list_certs(tenant);
    assert_eq!(before.certs.len(), 1);

    handle_remove_cert(tenant, "remove-tls");
    let after = handle_list_certs(tenant);
    assert!(after.certs.is_empty());
}
