// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-certs — CertificateRequest CRD + issuance lifecycle tests.
//!
//! Cite: cert-manager v1.13.0
//! `pkg/apis/certmanager/v1/types_certificaterequest.go`
//! CertificateRequest is the second core CRD (after Certificate) — it
//! carries a base64-encoded PEM CSR and tracks the issuance state.

use cave_certs::cert_request::{
    CertificateRequest, CertificateRequestSpec, CertificateRequestState, DenialReason,
};
use cave_certs::crds::IssuerRef;

const TENANT: &str = "tenant-certs-req";

fn spec(issuer: IssuerRef, csr_pem: &str) -> CertificateRequestSpec {
    CertificateRequestSpec {
        tenant_id: TENANT.into(),
        issuer_ref: issuer,
        csr_pem: csr_pem.into(),
        is_ca: false,
        duration_seconds: Some(90 * 86_400),
        usages: vec!["digital signature".into(), "server auth".into()],
    }
}

/// Cite: cert-manager `pkg/apis/certmanager/v1/types_certificaterequest.go`
/// — a new CertificateRequest starts in `Pending` state.
#[test]
fn cert_request_starts_pending() {
    let issuer = IssuerRef::issuer("letsencrypt-prod");
    let cr = CertificateRequest::new(TENANT, spec(issuer, "fake-csr-pem"));
    assert_eq!(cr.state, CertificateRequestState::Pending);
    assert!(cr.certificate_pem.is_none());
    assert!(cr.failure_message.is_none());
}

/// Cite: cert-manager CertificateRequest.spec.csr MUST be non-empty.
#[test]
fn cert_request_spec_validation_rejects_empty_csr() {
    let cr_spec = CertificateRequestSpec {
        tenant_id: TENANT.into(),
        issuer_ref: IssuerRef::issuer("le"),
        csr_pem: "".into(),
        is_ca: false,
        duration_seconds: None,
        usages: vec![],
    };
    assert!(cr_spec.validate().is_err());
}

/// Cite: cert-manager CertificateRequest.spec.tenantId MUST be non-empty.
#[test]
fn cert_request_spec_validation_rejects_empty_tenant() {
    let cr_spec = CertificateRequestSpec {
        tenant_id: "".into(),
        issuer_ref: IssuerRef::issuer("le"),
        csr_pem: "some-csr".into(),
        is_ca: false,
        duration_seconds: None,
        usages: vec![],
    };
    assert!(cr_spec.validate().is_err());
}

/// Cite: cert-manager controller — `approve()` transitions Pending → Approved;
/// `deny()` transitions Pending → Denied with a reason.
#[test]
fn cert_request_approve_and_deny_state_transitions() {
    let issuer = IssuerRef::issuer("letsencrypt-prod");
    let mut cr = CertificateRequest::new(TENANT, spec(issuer.clone(), "csr-1"));
    cr.approve();
    assert_eq!(cr.state, CertificateRequestState::Approved);

    // Deny a fresh request.
    let mut cr2 = CertificateRequest::new(TENANT, spec(issuer, "csr-2"));
    cr2.deny(DenialReason::PolicyViolation, "No wildcard certs allowed");
    assert_eq!(cr2.state, CertificateRequestState::Denied);
    assert!(cr2
        .failure_message
        .as_deref()
        .unwrap()
        .contains("wildcard"));
}

/// Cite: cert-manager controller — once Approved the issuer issues the cert
/// and calls `issue()` to stamp the PEM + move to Issued.
#[test]
fn cert_request_issue_stamps_pem_and_transitions_issued() {
    let issuer = IssuerRef::issuer("letsencrypt-prod");
    let mut cr = CertificateRequest::new(TENANT, spec(issuer, "csr-approved"));
    cr.approve();
    cr.issue("-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----\n");
    assert_eq!(cr.state, CertificateRequestState::Issued);
    assert!(cr
        .certificate_pem
        .as_deref()
        .unwrap()
        .starts_with("-----BEGIN CERTIFICATE-----"));
}

/// Cite: cert-manager — a CertificateRequest that has been Issued cannot
/// transition back to Pending or be denied.
#[test]
fn cert_request_already_issued_is_terminal() {
    let issuer = IssuerRef::issuer("letsencrypt-prod");
    let mut cr = CertificateRequest::new(TENANT, spec(issuer, "csr-done"));
    cr.approve();
    cr.issue("cert-pem");
    // Attempting to deny an already-issued request is a no-op / returns an error.
    let result = cr.try_deny(DenialReason::PolicyViolation, "too late");
    assert!(result.is_err(), "cannot deny an already-issued request");
    // State stays Issued.
    assert_eq!(cr.state, CertificateRequestState::Issued);
}
