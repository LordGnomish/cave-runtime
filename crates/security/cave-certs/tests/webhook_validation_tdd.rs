// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RED→GREEN TDD (2026-05-30): cert-manager webhook admission validation engine.
//!
//! Line-port of cert-manager v1.17.2
//! `internal/apis/certmanager/validation/certificate.go`
//! (`ValidateCertificateSpec`, `ValidateDuration`, `validateIssuerRef`,
//! `validateIPAddresses`, `validateEmailAddresses`, `validateUsages`).
//!
//! This is the PURE validation engine that cert-manager's webhook binary runs
//! on every admission request. The HTTP webhook transport / AdmissionReview
//! decode genuinely belongs in cave-admission, but the validation algorithm
//! itself is in-crate runtime logic.

use cave_certs::webhook_validation::{
    validate_certificate_spec, WebhookCertificateSpec, WebhookIssuerRef, WebhookKeyAlgorithm,
    WebhookPrivateKey,
};

fn base_spec() -> WebhookCertificateSpec {
    WebhookCertificateSpec {
        secret_name: "my-tls".into(),
        issuer_ref: WebhookIssuerRef {
            name: "le-prod".into(),
            kind: "Issuer".into(),
            group: "cert-manager.io".into(),
        },
        common_name: None,
        dns_names: vec!["example.com".into()],
        ip_addresses: vec![],
        email_addresses: vec![],
        uris: vec![],
        usages: vec![],
        private_key: None,
        duration_seconds: None,
        renew_before_seconds: None,
        is_ca: false,
        revision_history_limit: None,
    }
}

#[test]
fn valid_minimal_spec_has_no_errors() {
    let spec = base_spec();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.is_empty(), "expected no errors, got: {:?}", errs);
}

#[test]
fn empty_secret_name_is_required_error() {
    // Cite: certificate.go:47 — SecretName == "" => Required.
    let mut spec = base_spec();
    spec.secret_name = "".into();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.secretName"));
}

#[test]
fn secret_name_must_be_dns_subdomain() {
    // Cite: certificate.go:50 — NameIsDNSSubdomain.
    let mut spec = base_spec();
    spec.secret_name = "Invalid_Upper".into();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.secretName"));
}

#[test]
fn at_least_one_san_must_be_set() {
    // Cite: certificate.go:106-113.
    let mut spec = base_spec();
    spec.dns_names = vec![];
    spec.common_name = None;
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.message.contains("at least one of")));
}

#[test]
fn common_name_over_64_chars_too_long() {
    // Cite: certificate.go:116-118.
    let mut spec = base_spec();
    spec.common_name = Some("a".repeat(65));
    spec.dns_names = vec![];
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.commonName"));
}

#[test]
fn common_name_exactly_64_chars_ok() {
    let mut spec = base_spec();
    spec.common_name = Some("a".repeat(64));
    spec.dns_names = vec![];
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn invalid_ip_address_rejected() {
    // Cite: certificate.go:261-273 validateIPAddresses.
    let mut spec = base_spec();
    spec.ip_addresses = vec!["999.1.1.1".into(), "10.0.0.1".into()];
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs
        .iter()
        .any(|e| e.field == "spec.ipAddresses[0]" && e.message.contains("invalid IP")));
    // valid one produces no error
    assert!(!errs.iter().any(|e| e.field == "spec.ipAddresses[1]"));
}

#[test]
fn invalid_email_rejected_and_name_form_rejected() {
    // Cite: certificate.go:275-291 validateEmailAddresses.
    let mut spec = base_spec();
    spec.email_addresses = vec!["not-an-email".into(), "Alice <a@b.com>".into()];
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.emailAddresses[0]"));
    // RFC5322 name-form is rejected (Address != supplied value)
    assert!(errs.iter().any(|e| e.field == "spec.emailAddresses[1]"));
}

#[test]
fn rsa_key_size_out_of_range_rejected() {
    // Cite: certificate.go:150-153 — MinRSAKeySize=2048, MaxRSAKeySize=8192.
    let mut spec = base_spec();
    spec.private_key = Some(WebhookPrivateKey {
        algorithm: WebhookKeyAlgorithm::Rsa,
        size: 1024,
    });
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.privateKey.size"));
}

#[test]
fn ecdsa_unsupported_size_rejected() {
    // Cite: certificate.go:154-157 — only 256/384/521.
    let mut spec = base_spec();
    spec.private_key = Some(WebhookPrivateKey {
        algorithm: WebhookKeyAlgorithm::Ecdsa,
        size: 512,
    });
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.privateKey.size"));
}

#[test]
fn ecdsa_521_supported() {
    let mut spec = base_spec();
    spec.private_key = Some(WebhookPrivateKey {
        algorithm: WebhookKeyAlgorithm::Ecdsa,
        size: 521,
    });
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn duration_below_minimum_rejected() {
    // Cite: certificate.go:327-329 + ValidateDuration — MinimumCertificateDuration=1h.
    let mut spec = base_spec();
    spec.duration_seconds = Some(60); // 1 minute < 1h
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.duration"));
}

#[test]
fn renew_before_must_be_less_than_duration() {
    // Cite: certificate.go:342-344.
    let mut spec = base_spec();
    spec.duration_seconds = Some(3600 * 2);
    spec.renew_before_seconds = Some(3600 * 3); // >= duration
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.renewBefore"));
}

#[test]
fn renew_before_below_minimum_rejected() {
    // Cite: certificate.go:338-340 — MinimumRenewBefore=5m.
    let mut spec = base_spec();
    spec.duration_seconds = Some(3600 * 24);
    spec.renew_before_seconds = Some(60); // 1m < 5m minimum
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.renewBefore"));
}

#[test]
fn unknown_usage_rejected() {
    // Cite: certificate.go:293-303 validateUsages.
    let mut spec = base_spec();
    spec.usages = vec!["server auth".into(), "totally bogus".into()];
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.usages[1]"));
    assert!(!errs.iter().any(|e| e.field == "spec.usages[0]"));
}

#[test]
fn issuer_ref_name_required() {
    // Cite: certificate.go:223-226 validateIssuerRef.
    let mut spec = base_spec();
    spec.issuer_ref.name = "".into();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.issuerRef.name"));
}

#[test]
fn issuer_ref_bad_kind_with_builtin_group_rejected() {
    // Cite: certificate.go:228-256 — for cert-manager.io group, kind must be
    // Issuer or ClusterIssuer.
    let mut spec = base_spec();
    spec.issuer_ref.kind = "BogusKind".into();
    spec.issuer_ref.group = "cert-manager.io".into();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.issuerRef.kind"));
}

#[test]
fn issuer_ref_missing_group_with_external_kind_hint() {
    // Cite: certificate.go:244-251 — empty group + external kind => hint message.
    let mut spec = base_spec();
    spec.issuer_ref.kind = "AWSPCAClusterIssuer".into();
    spec.issuer_ref.group = "".into();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs
        .iter()
        .any(|e| e.field == "spec.issuerRef.kind" && e.message.contains("did you forget")));
}

#[test]
fn external_group_skips_kind_check() {
    // Cite: certificate.go:228 — non-builtin group => no kind validation.
    let mut spec = base_spec();
    spec.issuer_ref.kind = "AWSPCAClusterIssuer".into();
    spec.issuer_ref.group = "awspca.cert-manager.io".into();
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(!errs.iter().any(|e| e.field == "spec.issuerRef.kind"));
}

#[test]
fn revision_history_limit_below_one_rejected() {
    // Cite: certificate.go:171-173.
    let mut spec = base_spec();
    spec.revision_history_limit = Some(0);
    let errs = validate_certificate_spec(&spec, "spec");
    assert!(errs.iter().any(|e| e.field == "spec.revisionHistoryLimit"));
}
