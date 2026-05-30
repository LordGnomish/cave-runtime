// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! TDD coverage fills for cave-certs vs cert-manager v1.17.2
//! (https://github.com/cert-manager/cert-manager @ v1.17.2).
//!
//! These tests target two genuinely-uncovered, portable public cave functions:
//!
//! 1. `cave_certs::engine::days_until_expiry` — duration math mirroring
//!    cert-manager `pkg/util/pki` expiry helpers used by the trigger
//!    controller. Public, zero coverage anywhere in the crate.
//!
//! 2. `cave_certs::cert_request::CertificateRequest::try_deny` — the
//!    terminal-state guard from the cert-manager approval controller. The
//!    existing integration test only exercises the `Issued` error branch;
//!    here we cover the `Approved` and already-`Denied` error branches plus
//!    the happy `Pending` path (and its state mutation), which no existing
//!    test asserts.
//!
//! Expected values are derived directly from the cave implementation:
//!   - `days_until_expiry(cert) = (cert.not_after - Utc::now()).num_days()`,
//!     which truncates toward zero (whole days only, negative when expired).
//!   - `try_deny` returns `Ok(())` only from `Pending` (and flips the state to
//!     `Denied`); it returns `Err` from `Approved` / `Denied` / `Issued`
//!     without mutating state.

use cave_certs::cert_request::{
    CertificateRequest, CertificateRequestSpec, CertificateRequestState, DenialReason,
};
use cave_certs::crds::IssuerRef;
use cave_certs::engine::days_until_expiry;
use cave_certs::models::{CertState, Certificate};
use chrono::Utc;
use uuid::Uuid;

const TENANT: &str = "tenant-certs-tdd";

fn make_cert(not_after_days: i64) -> Certificate {
    let now = Utc::now();
    Certificate {
        id: Uuid::new_v4(),
        domain: "example.com".to_string(),
        san_domains: vec![],
        issuer: "Let's Encrypt".to_string(),
        not_before: now - chrono::Duration::days(60),
        not_after: now + chrono::Duration::days(not_after_days),
        serial_number: "01:AB".to_string(),
        fingerprint_sha256: "abc123".to_string(),
        state: CertState::Valid,
        auto_renew: true,
    }
}

fn make_spec() -> CertificateRequestSpec {
    CertificateRequestSpec {
        tenant_id: TENANT.into(),
        issuer_ref: IssuerRef::issuer("letsencrypt-prod"),
        csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\n...\n-----END CERTIFICATE REQUEST-----\n"
            .into(),
        is_ca: false,
        duration_seconds: Some(90 * 86_400),
        usages: vec!["server auth".into()],
    }
}

// ---------------------------------------------------------------------------
// engine::days_until_expiry
// ---------------------------------------------------------------------------

/// A certificate valid for 90 more days yields a positive whole-day count.
/// `num_days()` truncates toward zero, so a cert created `now + 90d` reports
/// 89 (a few elapsed nanoseconds shave the final partial day) or 90.
#[test]
fn days_until_expiry_future_cert_is_positive() {
    let cert = make_cert(90);
    let days = days_until_expiry(&cert);
    assert!(
        days == 89 || days == 90,
        "expected ~90 days for a 90-day cert, got {days}"
    );
}

/// An already-expired certificate (`not_after` 10 days in the past) reports a
/// negative count. Truncation toward zero gives -9 or -10.
#[test]
fn days_until_expiry_expired_cert_is_negative() {
    let cert = make_cert(-10);
    let days = days_until_expiry(&cert);
    assert!(
        days == -9 || days == -10,
        "expected ~-10 days for an expired cert, got {days}"
    );
}

/// A certificate expiring essentially "now" (`not_after == not_before-anchor`
/// + 0 days) is on the boundary: the elapsed clock means it has just crossed
/// into the past, so the truncated whole-day count is 0.
#[test]
fn days_until_expiry_boundary_is_zero() {
    let cert = make_cert(0);
    let days = days_until_expiry(&cert);
    assert_eq!(
        days, 0,
        "a cert expiring now should report 0 whole days remaining"
    );
}

// ---------------------------------------------------------------------------
// cert_request::CertificateRequest::try_deny — uncovered terminal branches
// ---------------------------------------------------------------------------

/// `try_deny` from `Pending` is the only success path: it returns `Ok(())`,
/// flips the state to `Denied`, and records a `[reason] message` failure note.
#[test]
fn try_deny_from_pending_succeeds_and_denies() {
    let mut cr = CertificateRequest::new(TENANT, make_spec());
    assert_eq!(cr.state, CertificateRequestState::Pending);

    let result = cr.try_deny(DenialReason::PolicyViolation, "no wildcards");
    assert!(result.is_ok(), "deny from Pending must succeed");
    assert_eq!(cr.state, CertificateRequestState::Denied);
    assert_eq!(
        cr.failure_message.as_deref(),
        Some("[PolicyViolation] no wildcards")
    );
}

/// `try_deny` on an `Approved` request is rejected with the
/// "revoke instead" guard and leaves the state untouched at `Approved`.
#[test]
fn try_deny_from_approved_errors_and_keeps_state() {
    let mut cr = CertificateRequest::new(TENANT, make_spec());
    cr.approve();
    assert_eq!(cr.state, CertificateRequestState::Approved);

    let result = cr.try_deny(DenialReason::PolicyViolation, "too late");
    assert!(result.is_err(), "cannot deny an already-approved request");
    assert_eq!(
        result.unwrap_err(),
        "cannot deny an already-approved CertificateRequest; revoke instead"
    );
    assert_eq!(cr.state, CertificateRequestState::Approved);
    // The failed try_deny must not stamp a failure message.
    assert!(cr.failure_message.is_none());
}

/// `try_deny` on an already-`Denied` request returns the "already denied"
/// error and does not overwrite the original denial message.
#[test]
fn try_deny_from_denied_errors_and_preserves_message() {
    let mut cr = CertificateRequest::new(TENANT, make_spec());
    // First denial succeeds and records its message.
    cr.try_deny(DenialReason::InvalidCsr, "bad csr").unwrap();
    assert_eq!(cr.state, CertificateRequestState::Denied);
    let first_msg = cr.failure_message.clone();
    assert_eq!(first_msg.as_deref(), Some("[InvalidCsr] bad csr"));

    // Second try_deny is rejected; original message is preserved.
    let result = cr.try_deny(DenialReason::Other, "again");
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        "CertificateRequest is already denied"
    );
    assert_eq!(cr.state, CertificateRequestState::Denied);
    assert_eq!(cr.failure_message, first_msg);
}
