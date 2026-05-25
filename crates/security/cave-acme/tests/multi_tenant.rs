// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-acme — multi-tenant ACME server tests.

use cave_acme::{AcmeError, AcmeServer, Identifier, Jwk, OrderStatus};

const TENANT_A: &str = "tenant-acme-prod";
const TENANT_B: &str = "tenant-beta-staging";

fn jwk(x: &str) -> Jwk {
    Jwk::OKP {
        crv: "Ed25519".into(),
        x: x.into(),
    }
}

/// Cite: RFC 8555 §7.3 — newAccount is idempotent on JWK thumbprint;
/// re-registration with the same key returns the original account id.
#[test]
fn new_account_dedupes_on_jwk_thumbprint() {
    let mut s = AcmeServer::new();
    let id1 = s
        .new_account(
            TENANT_A,
            jwk("aaaa"),
            vec![format!("mailto:ops@{}.test", TENANT_A)],
            true,
            None,
        )
        .unwrap();
    let id2 = s
        .new_account(
            TENANT_A,
            jwk("aaaa"),
            vec![format!("mailto:ops@{}.test", TENANT_A)],
            true,
            None,
        )
        .unwrap();
    assert_eq!(id1, id2);
    assert_eq!(s.account_count(), 1);

    // Same JWK under a DIFFERENT tenant ⇒ separate account.
    let id3 = s
        .new_account(
            TENANT_B,
            jwk("aaaa"),
            vec![format!("mailto:ops@{}.test", TENANT_B)],
            true,
            None,
        )
        .unwrap();
    assert_ne!(id1, id3);
    assert_eq!(s.account_count(), 2);
}

/// Cite: RFC 8555 §7.4 + cave multi-tenant invariant — orders are
/// scoped to the tenant that created them. A different tenant
/// requesting the same order id receives `CrossTenantDenied`.
#[test]
fn orders_are_tenant_scoped_and_block_cross_tenant_lookup() {
    let mut s = AcmeServer::new();
    let acct_a = s
        .new_account(
            TENANT_A,
            jwk("aaaa"),
            vec![format!("mailto:ops@{}.test", TENANT_A)],
            true,
            None,
        )
        .unwrap();
    let order_id = s
        .new_order(
            TENANT_A,
            &acct_a,
            vec![Identifier::dns(format!(
                "svc.{}.cave-runtime.test",
                TENANT_A
            ))],
        )
        .unwrap();
    assert_eq!(s.order_count(), 1);
    assert_eq!(s.authorization_count(), 1);

    // Owner can read the order
    let order = s.order(TENANT_A, &order_id).unwrap();
    assert_eq!(order.status, OrderStatus::Pending);
    assert_eq!(order.tenant_id, TENANT_A);

    // Cross-tenant read MUST fail
    let err = s.order(TENANT_B, &order_id).unwrap_err();
    assert!(matches!(err, AcmeError::CrossTenantDenied { .. }));

    // Cross-tenant finalize MUST also fail
    let err = s
        .finalize_order(TENANT_B, &order_id, "https://x")
        .unwrap_err();
    assert!(matches!(err, AcmeError::CrossTenantDenied { .. }));
}
