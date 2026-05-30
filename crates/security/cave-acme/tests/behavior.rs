// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Behavioral integration tests for `cave_acme::server::AcmeServer`.
//!
//! These port server-side ACME state-transition behaviors from
//! smallstep/certificates v0.30.2 (`acme/` package — `TestHandler_NewOrder`,
//! `TestHandler_newAuthorization`, `TestHandler_FinalizeOrder`,
//! `TestHandler_GetOrUpdateAccount`, `TestDB_GetAccount`). cave-acme is the
//! RFC 8555 §7 in-memory state machine; each test drives an `AcmeServer` with
//! no network or persistence and asserts the concrete status / error variant
//! the impl in `src/server.rs` produces.
//!
//! Note: the gap report also lists `mark_challenge_valid` driven transitions
//! (challenge→authz→order Ready, the all-valid gate, and finalize→Valid).
//! Those are intentionally NOT covered here: `AcmeServer` stores its
//! `Authorization`/`Challenge` objects privately and exposes no public getter
//! for challenge ids, so `mark_challenge_valid(challenge_id)` cannot be reached
//! through the public crate surface from an integration test. Testing it would
//! require either a new public accessor or reaching into private state — both
//! out of scope for a black-box behavior test.

use cave_acme::account::Jwk;
use cave_acme::error::AcmeError;
use cave_acme::order::{Identifier, OrderStatus};
use cave_acme::server::AcmeServer;

/// Deterministic EC JWK. Only thumbprint stability matters; these tests use a
/// single account so the exact key material is irrelevant.
fn jwk() -> Jwk {
    Jwk::EC {
        crv: "P-256".into(),
        x: "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU".into(),
        y: "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0".into(),
    }
}

/// Create one valid account; return (server, tenant, account_id).
fn server_with_account() -> (AcmeServer, String, String) {
    let mut srv = AcmeServer::new();
    let tenant = "tenant-a".to_string();
    let acct = srv
        .new_account(
            tenant.clone(),
            jwk(),
            vec!["mailto:admin@example.com".into()],
            true,
            None,
        )
        .expect("new_account succeeds for a valid request");
    (srv, tenant, acct)
}

#[test]
fn account_lookup_unknown_id_is_not_found() {
    // Cite: smallstep TestDB_GetAccount (not-found path). An id never created
    // yields AccountNotFound carrying that exact id (NOT a cross-tenant error,
    // because the not-found check runs before the tenant check).
    let (srv, tenant, _acct) = server_with_account();
    let missing = "00000000-0000-0000-0000-000000000000";
    let err = srv
        .account(&tenant, missing)
        .expect_err("unknown account id must error");
    assert_eq!(err, AcmeError::AccountNotFound(missing.to_string()));
}

#[test]
fn account_lookup_known_id_returns_account() {
    // Cite: smallstep TestDB_GetAccount (found path). A created account is
    // retrievable by id under its owning tenant and carries Valid status.
    let (srv, tenant, acct) = server_with_account();
    let got = srv.account(&tenant, &acct).expect("account is retrievable");
    assert_eq!(got.id, acct);
    assert_eq!(got.tenant_id, tenant);
    assert_eq!(got.status, cave_acme::account::AccountStatus::Valid);
}

#[test]
fn new_order_creates_authz_per_identifier_and_stays_pending() {
    // Cite: smallstep TestHandler_NewOrder / newAuthorization. new_order fans
    // out exactly one Authorization per identifier and the fresh order starts
    // in Pending with its authorization_ids populated 1:1 with identifiers.
    let (mut srv, tenant, acct) = server_with_account();
    let order_id = srv
        .new_order(
            &tenant,
            &acct,
            vec![
                Identifier::dns("a.example.com"),
                Identifier::dns("b.example.com"),
            ],
        )
        .expect("two-identifier order creates");

    // Two identifiers => two authorizations registered server-wide.
    assert_eq!(srv.authorization_count(), 2);
    assert_eq!(srv.order_count(), 1);

    let order = srv.order(&tenant, &order_id).unwrap();
    assert_eq!(order.identifiers.len(), 2);
    assert_eq!(order.authorization_ids.len(), 2);
    assert_eq!(order.status, OrderStatus::Pending);
    // Finalize URL is derived deterministically from the order id.
    assert_eq!(order.finalize_url, format!("/acme/order/{}/finalize", order_id));
    // Not yet finalized: no certificate URL.
    assert_eq!(order.certificate_url, None);
}

#[test]
fn new_order_rejects_uppercase_dns_identifier() {
    // Cite: smallstep newOrder identifier canonicalization. validate_identifiers
    // (called inside new_order) rejects a non-lowercase DNS identifier with
    // Malformed, and the order is NOT persisted.
    let (mut srv, tenant, acct) = server_with_account();
    let err = srv
        .new_order(&tenant, &acct, vec![Identifier::dns("Example.COM")])
        .expect_err("uppercase DNS identifier must be rejected");
    assert_eq!(
        err,
        AcmeError::Malformed("DNS identifier 'Example.COM' must be lowercase".to_string())
    );
    // Rejected before insertion: no order stored.
    assert_eq!(srv.order_count(), 0);
}

#[test]
fn new_order_rejects_empty_identifiers() {
    // Cite: smallstep newOrder empty-identifier guard. An order with no
    // identifiers is Malformed and not persisted.
    let (mut srv, tenant, acct) = server_with_account();
    let err = srv
        .new_order(&tenant, &acct, vec![])
        .expect_err("empty identifier list must be rejected");
    assert_eq!(
        err,
        AcmeError::Malformed("order has no identifiers".to_string())
    );
    assert_eq!(srv.order_count(), 0);
}

#[test]
fn deactivated_account_cannot_create_order() {
    // Cite: smallstep TestHandler_GetOrUpdateAccount (deactivate). After
    // deactivate_account, new_order rejects with Unauthorized because the guard
    // requires AccountStatus::Valid; the message embeds the Debug status.
    let (mut srv, tenant, acct) = server_with_account();
    srv.deactivate_account(&tenant, &acct)
        .expect("deactivate succeeds");

    let err = srv
        .new_order(&tenant, &acct, vec![Identifier::dns("example.com")])
        .expect_err("deactivated account must not create orders");
    assert_eq!(
        err,
        AcmeError::Unauthorized(format!("account {} is Deactivated", acct))
    );
    assert_eq!(srv.order_count(), 0);
}

#[test]
fn finalize_order_rejects_non_ready_order() {
    // Cite: smallstep TestHandler_FinalizeOrder (status guard). A freshly
    // created order is Pending; finalize rejects with OrderNotReady whose
    // second field is the Debug-rendered current status "Pending", and the
    // guard returns before stamping the certificate URL.
    let (mut srv, tenant, acct) = server_with_account();
    let order_id = srv
        .new_order(&tenant, &acct, vec![Identifier::dns("example.com")])
        .expect("order creates");

    let err = srv
        .finalize_order(&tenant, &order_id, "/acme/cert/should-not-stamp")
        .expect_err("finalize of a Pending order must error");
    assert_eq!(
        err,
        AcmeError::OrderNotReady(order_id.clone(), "Pending".to_string())
    );

    // Guard returned early: status unchanged, no certificate URL stamped.
    let order = srv.order(&tenant, &order_id).unwrap();
    assert_eq!(order.status, OrderStatus::Pending);
    assert_eq!(order.certificate_url, None);
}

#[test]
fn finalize_order_unknown_id_is_malformed() {
    // Cite: smallstep finalize lookup. Finalizing an order id that was never
    // created yields Malformed("order <id> not found").
    let (mut srv, tenant, _acct) = server_with_account();
    let missing = "11111111-2222-3333-4444-555555555555";
    let err = srv
        .finalize_order(&tenant, missing, "/acme/cert/x")
        .expect_err("unknown order id must error");
    assert_eq!(
        err,
        AcmeError::Malformed(format!("order {} not found", missing))
    );
}

#[test]
fn order_lookup_cross_tenant_is_denied() {
    // Cite: cave-acme multi-tenant invariant on the order getter. A second
    // tenant requesting another tenant's order id gets CrossTenantDenied with
    // the stored vs requesting tenant recorded.
    let (mut srv, tenant, acct) = server_with_account();
    let order_id = srv
        .new_order(&tenant, &acct, vec![Identifier::dns("example.com")])
        .expect("order creates");

    let err = srv
        .order("tenant-b", &order_id)
        .expect_err("cross-tenant order lookup must be denied");
    assert_eq!(
        err,
        AcmeError::CrossTenantDenied {
            store: tenant.clone(),
            req: "tenant-b".to_string(),
        }
    );
}
