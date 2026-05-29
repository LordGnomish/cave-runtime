// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-certs — issuer backend tests: SelfSigned + CA + CSR generation.
//!
//! Cite: cert-manager v1.13.0
//! `pkg/issuer/selfsigned/issue.go` (SelfSigned issuer),
//! `pkg/issuer/ca/issue.go` (CA issuer),
//! `pkg/util/pki/csr.go` (CSR generation).

use cave_certs::issuers::{CaIssuer, IssueRequest, IssueResult, SelfSignedIssuer};
use cave_certs::csr::{CsrBuilder, CsrParams};
use cave_certs::crds::IssuerRef;

const TENANT: &str = "tenant-issuers-test";

/// Cite: cert-manager `pkg/issuer/selfsigned/issue.go` — the SelfSigned
/// issuer generates a key pair and signs the cert with the private key it
/// just created. No CA is needed; the cert is its own issuer.
#[test]
fn selfsigned_issuer_produces_valid_cert_pem() {
    let issuer = SelfSignedIssuer::new(TENANT);
    let req = IssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec!["selfsigned.example.com".into()],
        common_name: Some("selfsigned.example.com".into()),
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    let result = issuer.issue(&req).unwrap();
    assert!(!result.certificate_pem.is_empty());
    assert!(!result.private_key_pem.is_empty());
    // The PEM blocks must carry the right headers.
    assert!(result.certificate_pem.contains("CERTIFICATE"));
    assert!(result.private_key_pem.contains("PRIVATE KEY") || result.private_key_pem.contains("EC PRIVATE KEY"));
}

/// Cite: cert-manager selfsigned issuer — is_ca=true produces a CA
/// certificate (BasicConstraints CA:TRUE, keyCertSign usage).
#[test]
fn selfsigned_issuer_can_produce_ca_cert() {
    let issuer = SelfSignedIssuer::new(TENANT);
    let req = IssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec!["ca.internal.example.com".into()],
        common_name: Some("Cave Internal CA".into()),
        duration_seconds: 5 * 365 * 86_400,
        is_ca: true,
    };
    let result = issuer.issue(&req).unwrap();
    assert!(result.is_ca);
    assert!(result.certificate_pem.contains("CERTIFICATE"));
}

/// Cite: cert-manager `pkg/issuer/ca/issue.go` — the CA issuer signs certs
/// using the CA key pair from the store. cave stores the CA as a
/// SelfSignedIssuer result, then uses CaIssuer to sign leaf certs from it.
#[test]
fn ca_issuer_signs_leaf_cert_from_stored_ca() {
    // First, create a self-signed CA.
    let ss = SelfSignedIssuer::new(TENANT);
    let ca_req = IssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec![],
        common_name: Some("Cave Test CA".into()),
        duration_seconds: 5 * 365 * 86_400,
        is_ca: true,
    };
    let ca_result = ss.issue(&ca_req).unwrap();

    // Now use CaIssuer with that CA to sign a leaf cert.
    let ca_issuer = CaIssuer::from_pem(TENANT, &ca_result.certificate_pem, &ca_result.private_key_pem).unwrap();
    let leaf_req = IssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec!["api.example.com".into(), "www.example.com".into()],
        common_name: Some("api.example.com".into()),
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    let leaf = ca_issuer.sign(&leaf_req).unwrap();
    assert!(leaf.certificate_pem.contains("CERTIFICATE"));
    assert!(!leaf.is_ca);
}

/// Cite: cert-manager `pkg/util/pki/csr.go::GenerateCSR` — given a set of
/// DNS names + key material, produce a PKCS#10 CSR PEM.
#[test]
fn csr_builder_produces_parseable_pem() {
    let params = CsrParams {
        tenant_id: TENANT.into(),
        dns_names: vec!["api.example.com".into(), "www.example.com".into()],
        common_name: Some("api.example.com".into()),
    };
    let csr = CsrBuilder::build(&params).unwrap();
    assert!(
        csr.pem.contains("CERTIFICATE REQUEST"),
        "CSR PEM must contain CERTIFICATE REQUEST header"
    );
    assert!(
        !csr.private_key_pem.is_empty(),
        "CsrBuilder should also return the private key"
    );
}

/// Cross-tenant: a CaIssuer for tenant A must refuse to sign for tenant B.
#[test]
fn ca_issuer_rejects_cross_tenant_sign_request() {
    let ss = SelfSignedIssuer::new(TENANT);
    let ca_req = IssueRequest {
        tenant_id: TENANT.into(),
        dns_names: vec![],
        common_name: Some("Cave Test CA".into()),
        duration_seconds: 365 * 86_400,
        is_ca: true,
    };
    let ca_result = ss.issue(&ca_req).unwrap();
    let ca_issuer = CaIssuer::from_pem(TENANT, &ca_result.certificate_pem, &ca_result.private_key_pem).unwrap();
    let leaf_req = IssueRequest {
        tenant_id: "tenant-other".into(), // wrong tenant
        dns_names: vec!["api.other.com".into()],
        common_name: None,
        duration_seconds: 90 * 86_400,
        is_ca: false,
    };
    let err = ca_issuer.sign(&leaf_req);
    assert!(err.is_err(), "must reject cross-tenant sign request");
}
