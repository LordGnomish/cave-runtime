// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! cave-acme — Order + Authorization tests pinned to RFC 8555.

use cave_acme::{Identifier, IdentifierType, Order, OrderStatus};

const TENANT: &str = "tenant-acme-prod";

/// Cite: RFC 8555 §7.4 — newOrder rejects empty identifier list and
/// requires lowercase DNS names (case-insensitive matching).
#[test]
fn order_validate_identifiers_rejects_empty_and_uppercase() {
    let mut o = Order::new("ord-1", TENANT, "acct-1", vec![]);
    assert!(o.validate_identifiers().is_err());

    o.identifiers = vec![Identifier {
        kind: IdentifierType::Dns,
        value: "API.Example.com".into(),
    }];
    let err = o.validate_identifiers().unwrap_err();
    assert!(err.to_string().contains("must be lowercase"));

    o.identifiers = vec![Identifier::dns(format!("api.{}.cave-runtime.test", TENANT))];
    assert!(o.validate_identifiers().is_ok());
}

/// Cite: RFC 8555 §7.1.6 — order status state machine. Allowed:
/// `pending → ready → processing → valid` (and `invalid` from any
/// non-terminal state). `valid` and `invalid` are terminal sinks.
#[test]
fn order_status_state_machine_allowed_transitions() {
    use OrderStatus::*;
    // Forward path
    assert!(Pending.can_transition_to(Ready));
    assert!(Ready.can_transition_to(Processing));
    assert!(Processing.can_transition_to(Valid));
    // Invalid is reachable from each non-terminal state
    assert!(Pending.can_transition_to(Invalid));
    assert!(Ready.can_transition_to(Invalid));
    assert!(Processing.can_transition_to(Invalid));
    // Self-loops (idempotent re-set) are allowed.
    assert!(Pending.can_transition_to(Pending));
    assert!(Valid.can_transition_to(Valid));
    // Forbidden: backward + cross-skip + reviving terminals.
    assert!(!Ready.can_transition_to(Pending));
    assert!(!Pending.can_transition_to(Processing));
    assert!(!Pending.can_transition_to(Valid));
    assert!(!Valid.can_transition_to(Pending));
    assert!(!Valid.can_transition_to(Ready));
    assert!(!Invalid.can_transition_to(Pending));
}

/// Cite: RFC 8555 §7.4 — `Order.expires` is a future timestamp;
/// finalize URL is derived from the order id.
#[test]
fn order_new_default_fields() {
    let o = Order::new(
        "ord-tenant",
        TENANT,
        "acct-tenant",
        vec![Identifier::dns(format!("svc.{}.cave-runtime.test", TENANT))],
    );
    assert_eq!(o.tenant_id, TENANT);
    assert_eq!(o.account_id, "acct-tenant");
    assert_eq!(o.status, OrderStatus::Pending);
    assert!(o.expires > chrono::Utc::now());
    assert_eq!(o.finalize_url, "/acme/order/ord-tenant/finalize");
    assert!(o.certificate_url.is_none());
    assert!(!o.is_terminal());
}
