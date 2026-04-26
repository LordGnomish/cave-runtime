//! cave-pki — CRL responder tests pinned to RFC 5280 §5.

use cave_pki::{CrlResponder, RevocationReason};

const TENANT_A: &str = "tenant-acme-prod";
const TENANT_B: &str = "tenant-beta-staging";

/// Cite: RFC 5280 §5.3.1 — every CRLReason has a fixed numeric code.
/// Code 7 is unassigned per the RFC; cave's `from_code` reflects that.
#[test]
fn revocation_reason_codes_match_rfc5280_table() {
    use RevocationReason::*;
    assert_eq!(Unspecified.code(), 0);
    assert_eq!(KeyCompromise.code(), 1);
    assert_eq!(CaCompromise.code(), 2);
    assert_eq!(AffiliationChanged.code(), 3);
    assert_eq!(Superseded.code(), 4);
    assert_eq!(CessationOfOperation.code(), 5);
    assert_eq!(CertificateHold.code(), 6);
    assert_eq!(RemoveFromCrl.code(), 8);   // 7 is reserved/unassigned
    assert_eq!(PrivilegeWithdrawn.code(), 9);
    assert_eq!(AaCompromise.code(), 10);

    assert_eq!(RevocationReason::from_code(0), Some(Unspecified));
    assert_eq!(RevocationReason::from_code(7), None, "code 7 is reserved");
    assert_eq!(RevocationReason::from_code(99), None);
}

/// Cite: openbao `pki/path_revoke.go` — revoking a serial twice is
/// idempotent (entry overwritten, never duplicated). RemoveFromCrl
/// (operator-driven unrevoke) succeeds.
#[test]
fn revoke_unrevoke_is_idempotent() {
    let mut crl = CrlResponder::new();
    let serial = "DEADBEEFCAFEBABE0123";
    let entry1 = crl.revoke(serial, RevocationReason::KeyCompromise, TENANT_A);
    assert_eq!(crl.len(), 1);
    let entry2 = crl.revoke(serial, RevocationReason::Superseded, TENANT_A);
    assert_eq!(crl.len(), 1, "duplicate revoke is overwrite, not insert");
    assert_eq!(entry1.serial, entry2.serial);
    assert_eq!(crl.lookup(serial).unwrap().reason, RevocationReason::Superseded);

    assert!(crl.unrevoke(serial));
    assert!(crl.lookup(serial).is_none());
    assert!(!crl.unrevoke(serial), "second unrevoke is a no-op");

    assert!(crl.last_rebuilt_at.is_some(),
        "every mutation stamps last_rebuilt_at (RFC 5280 §5.1.2.1 thisUpdate)");
}

/// Cite: cave multi-tenant invariant — `for_tenant(t)` only returns
/// entries owned by tenant `t`.
#[test]
fn tenant_scoped_revocations_dont_leak_across_tenants() {
    let mut crl = CrlResponder::new();
    crl.revoke("AAAA0001", RevocationReason::KeyCompromise, TENANT_A);
    crl.revoke("AAAA0002", RevocationReason::Superseded, TENANT_A);
    crl.revoke("BBBB0003", RevocationReason::CessationOfOperation, TENANT_B);

    let a = crl.for_tenant(TENANT_A);
    let b = crl.for_tenant(TENANT_B);
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 1);
    assert!(a.iter().all(|e| e.tenant_id == TENANT_A));
    assert_eq!(crl.for_tenant("tenant-nobody").len(), 0);
}
