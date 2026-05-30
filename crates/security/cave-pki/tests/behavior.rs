// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-pki — portable-coverage behavioral tests.
//!
//! These assertions port upstream smallstep/certificates v0.30.2 behaviors
//! (RFC 5280 path validation, RFC 5280 §5.3.1 CRLReason table, RFC 6960
//! OCSP status semantics) onto cave-pki's deliberately small PKI core. Each
//! test exercises a public `cave_pki::*` item that is already implemented in
//! src but lacked an asserting test, and checks a concrete value derived from
//! the implementation logic (exact enum variant, exact ordering, exact error).

use cave_pki::{
    Ca, CaKind, ChainValidator, CrlResponder, KeyAlgorithm, OcspResponder, OcspStatus, PkiError,
    RevocationReason, ValidationResult,
};
use chrono::{Duration, Utc};
use std::collections::HashSet;

const TENANT: &str = "tenant-acme-prod";

/// Build a well-formed Root -> Platform -> Tenant hierarchy and return the
/// CA plus the leaf (tenant intermediate) serial.
fn primed_ca() -> (Ca, String) {
    let mut ca = Ca::new();
    ca.generate_root("Cave Sovereign Root", KeyAlgorithm::EcdsaP384, 20)
        .unwrap();
    ca.generate_platform_intermediate("Cave Platform CA", KeyAlgorithm::EcdsaP384)
        .unwrap();
    let tenant_serial = ca
        .generate_tenant_intermediate(TENANT, KeyAlgorithm::EcdsaP256)
        .unwrap();
    (ca, tenant_serial)
}

/// Cite: RFC 5280 §6.1.3 — a certificate whose `notBefore` is in the future
/// relative to the validation instant must fail. Every cave handle is created
/// with `not_before == Utc::now()`, so a validator clock set 10 days earlier
/// trips the `not_before > now` branch in `ChainValidator::validate`
/// (chain.rs:65) before any expiry or parent-link check.
#[test]
fn chain_rejects_not_yet_valid_cert() {
    let (ca, tenant_serial) = primed_ca();
    let validator = ChainValidator::new(&ca).at(Utc::now() - Duration::days(10));
    match validator.validate(&tenant_serial).unwrap() {
        ValidationResult::Invalid(reason) => {
            assert!(
                reason.contains("not yet valid"),
                "expected a not-yet-valid reason, got: {reason}"
            );
        }
        other => panic!("expected Invalid(not yet valid), got {other:?}"),
    }
}

/// Cite: path-lookup error — `Ca::chain_for` (ca.rs:217) on a serial that was
/// never issued returns the exact `PkiError::ParentNotFound` variant carrying
/// the queried serial.
#[test]
fn chain_for_unknown_serial_is_parent_not_found() {
    let (ca, _tenant_serial) = primed_ca();
    let err = ca.chain_for("nonexistent-serial-0000").unwrap_err();
    assert_eq!(
        err,
        PkiError::ParentNotFound("nonexistent-serial-0000".into())
    );
}

/// Cite: RFC 5246 §7.4.2 — TLS expects a leaf-first chain. `Ca::chain_for`
/// (ca.rs:217) walks issuer links and must yield Tenant -> Platform -> Root in
/// that order, with the root carrying no issuer reference.
#[test]
fn chain_for_returns_leaf_first_root_last_ordering() {
    let (ca, tenant_serial) = primed_ca();
    let chain = ca.chain_for(&tenant_serial).unwrap();

    assert_eq!(chain.len(), 3, "tenant + platform + root");
    assert_eq!(chain[0].kind, CaKind::TenantIntermediate);
    assert_eq!(chain[1].kind, CaKind::PlatformIntermediate);
    assert_eq!(chain[2].kind, CaKind::Root);

    assert_eq!(chain[0].serial, tenant_serial, "leaf is first");
    assert_eq!(
        chain[2].serial,
        ca.root_serial().unwrap(),
        "root is last"
    );
    assert_eq!(
        chain[2].issuer_serial, None,
        "root is self-anchored (no issuer reference)"
    );
    // Each non-root element references the next element up as its issuer.
    assert_eq!(chain[0].issuer_serial.as_deref(), Some(chain[1].serial.as_str()));
    assert_eq!(chain[1].issuer_serial.as_deref(), Some(chain[2].serial.as_str()));
}

/// Cite: RFC 5280 §5.3.1 + acme/api/revoke_test.go::Test_validateReasonCode —
/// the CRLReason numeric table round-trips for every assigned variant, while
/// the reserved code 7 and all codes 11..=255 have no mapping. Exercises
/// `RevocationReason::code` / `from_code` (crl.rs:31-49) across the full enum,
/// not just literal-by-literal spot checks.
#[test]
fn revocation_reason_code_roundtrip_total() {
    use RevocationReason::*;
    let all = [
        Unspecified,
        KeyCompromise,
        CaCompromise,
        AffiliationChanged,
        Superseded,
        CessationOfOperation,
        CertificateHold,
        RemoveFromCrl,
        PrivilegeWithdrawn,
        AaCompromise,
    ];
    for r in all {
        assert_eq!(
            RevocationReason::from_code(r.code()),
            Some(r),
            "code<->from_code must round-trip for {r:?}"
        );
    }

    // Code 7 is reserved/unassigned per RFC 5280.
    assert_eq!(RevocationReason::from_code(7), None, "code 7 is reserved");
    // Everything above the assigned range (11..=255) is also unmapped.
    for c in 11u8..=255 {
        assert_eq!(
            RevocationReason::from_code(c),
            None,
            "code {c} is outside the assigned CRLReason range"
        );
    }
}

/// Cite: RFC 6960 §2.2 / §4.2.1 — `good` only attests "not revoked at request
/// time"; authority is governed by issuer membership (cave's `known_serials`).
/// A serial that is neither revoked nor in `known_serials` yields `Unknown`;
/// adding it to `known_serials` flips the very same lookup to `Good`. This
/// isolates that the Good/Unknown discrimination in `OcspResponder::check`
/// (ocsp.rs:58) is decided purely by membership, independent of revocation.
#[test]
fn ocsp_unknown_for_serial_outside_authority() {
    let crl = CrlResponder::new();
    let mut known: HashSet<String> = HashSet::new();

    let resp = OcspResponder::new(&crl, &known);
    assert_eq!(
        resp.check("WORKLOAD-7F3A"),
        OcspStatus::Unknown,
        "serial outside this responder's authority is Unknown, not Good"
    );
    drop(resp);

    known.insert("WORKLOAD-7F3A".into());
    let resp = OcspResponder::new(&crl, &known);
    assert_eq!(
        resp.check("WORKLOAD-7F3A"),
        OcspStatus::Good,
        "same non-revoked serial becomes Good once it is within authority"
    );
}

/// Cite: api/crl_test.go::Test_CRL — `CrlResponder` membership accessors
/// (`is_revoked` / `len` / `is_empty`, crl.rs:112-121) must track `revoke`
/// and `unrevoke` exactly.
#[test]
fn crl_is_revoked_len_is_empty_track_membership() {
    let mut crl = CrlResponder::new();
    let serial = "DEADBEEFCAFEBABE0123";

    assert!(crl.is_empty());
    assert_eq!(crl.len(), 0);
    assert!(!crl.is_revoked(serial));

    crl.revoke(serial, RevocationReason::KeyCompromise, TENANT);
    assert!(crl.is_revoked(serial));
    assert!(!crl.is_empty());
    assert_eq!(crl.len(), 1);
    // An unrelated serial is still not revoked.
    assert!(!crl.is_revoked("UNRELATED-0000"));

    assert!(crl.unrevoke(serial), "unrevoke of a known serial succeeds");
    assert!(!crl.is_revoked(serial));
    assert!(crl.is_empty());
    assert_eq!(crl.len(), 0);
}

/// Cite: RFC 6960 §2.2 — when a serial is both within authority
/// (`known_serials`) and present in the CRL, `revoked` MUST win:
/// `OcspResponder::check` (ocsp.rs:51) consults the CRL before the
/// `known_serials` membership test, so the answer is `Revoked` (with the
/// stored reason), never `Good`.
#[test]
fn ocsp_revoked_takes_precedence_over_known_membership() {
    let mut crl = CrlResponder::new();
    let known: HashSet<String> = ["XYZ0001".into()].into_iter().collect();
    crl.revoke("XYZ0001", RevocationReason::CaCompromise, TENANT);

    let resp = OcspResponder::new(&crl, &known);
    match resp.check("XYZ0001") {
        OcspStatus::Revoked { reason, .. } => {
            assert_eq!(reason, RevocationReason::CaCompromise);
        }
        other => panic!("revocation must override known-membership Good, got {other:?}"),
    }
}
