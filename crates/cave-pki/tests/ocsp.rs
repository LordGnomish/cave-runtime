// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-pki — OCSP responder tests pinned to RFC 6960.

use cave_pki::{CrlResponder, OcspResponder, OcspStatus, RevocationReason};
use std::collections::HashSet;

const TENANT: &str = "tenant-acme-prod";

/// Cite: RFC 6960 §2.2 — `certStatus` is one of `good`, `revoked`,
/// `unknown`. Unknown is returned for serials the responder has no
/// authority over (cave models this with `known_serials` membership).
#[test]
fn ocsp_returns_good_revoked_unknown_per_status_table() {
    let mut crl = CrlResponder::new();
    let mut known: HashSet<String> = HashSet::new();
    known.insert("AAAA0001".into());
    known.insert("AAAA0002".into());
    crl.revoke("AAAA0002", RevocationReason::KeyCompromise, TENANT);

    let resp = OcspResponder::new(&crl, &known);
    assert_eq!(resp.check("AAAA0001"), OcspStatus::Good);
    assert!(matches!(resp.check("AAAA0002"),
        OcspStatus::Revoked { reason: RevocationReason::KeyCompromise, .. }));
    assert_eq!(resp.check("ZZZZ-not-our-issuer"), OcspStatus::Unknown);
}

/// Cite: RFC 6960 §4.2.1 — `revoked` MUST carry the revocation time and
/// (optionally) the reason. cave's responder pulls both from the CRL.
#[test]
fn ocsp_revoked_response_carries_time_and_reason() {
    let mut crl = CrlResponder::new();
    let known: HashSet<String> = ["XYZ0001".into()].into_iter().collect();
    let entry = crl.revoke("XYZ0001", RevocationReason::CaCompromise, TENANT);

    let resp = OcspResponder::new(&crl, &known);
    match resp.check("XYZ0001") {
        OcspStatus::Revoked { revoked_at, reason } => {
            assert_eq!(revoked_at, entry.revoked_at);
            assert_eq!(reason, RevocationReason::CaCompromise);
        }
        other => panic!("expected Revoked, got {:?}", other),
    }
}
