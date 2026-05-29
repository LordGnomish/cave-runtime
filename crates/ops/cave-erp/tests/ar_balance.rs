// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Integration tests for accounts-receivable balance computation.
//! These tests exercise cave_erp::ar which does not yet exist.

use cave_erp::ar::{compute_partner_balance, PartnerBalance, BalanceDirection};
use cave_erp::models::{
    Invoice, InvoiceKind, InvoiceState, InvoiceLine, Payment, PaymentMethod, PaymentState,
};
use chrono::Utc;
use uuid::Uuid;

fn make_invoice(partner_id: Uuid, amount: f64, state: InvoiceState) -> Invoice {
    Invoice {
        id: Uuid::new_v4(),
        number: format!("INV-{}", Uuid::new_v4()),
        partner_id,
        kind: InvoiceKind::Customer,
        journal_id: Uuid::new_v4(),
        lines: vec![InvoiceLine {
            id: Uuid::new_v4(),
            product_id: None,
            description: "Service".to_string(),
            quantity: 1.0,
            unit_price: amount,
            tax_ids: vec![],
            subtotal: amount,
        }],
        amount_total: amount,
        state,
        due_date: Utc::now(),
        created_at: Utc::now(),
    }
}

fn make_payment(invoice_id: Uuid, amount: f64) -> Payment {
    Payment {
        id: Uuid::new_v4(),
        invoice_id,
        amount,
        date: Utc::now(),
        method: PaymentMethod::Bank,
        state: PaymentState::Done,
        created_at: Utc::now(),
    }
}

#[test]
fn test_partner_balance_no_invoices_is_zero() {
    let partner_id = Uuid::new_v4();
    let balance = compute_partner_balance(partner_id, &[], &[]);
    assert!((balance.outstanding - 0.0).abs() < 0.01);
    assert_eq!(balance.direction, BalanceDirection::None);
}

#[test]
fn test_partner_balance_with_posted_invoice_and_no_payment() {
    let partner_id = Uuid::new_v4();
    let inv = make_invoice(partner_id, 1000.0, InvoiceState::Posted);
    let inv_id = inv.id;

    let balance = compute_partner_balance(partner_id, &[inv], &[]);

    // 1000 outstanding, no payments yet
    assert!((balance.outstanding - 1000.0).abs() < 0.01, "outstanding={}", balance.outstanding);
    assert_eq!(balance.direction, BalanceDirection::Receivable);
    assert_eq!(balance.invoice_count, 1);
    assert!((balance.paid - 0.0).abs() < 0.01);
    let _ = inv_id; // silence warning
}

#[test]
fn test_partner_balance_partial_payment() {
    let partner_id = Uuid::new_v4();
    let inv = make_invoice(partner_id, 1000.0, InvoiceState::Posted);
    let payment = make_payment(inv.id, 400.0);

    let balance = compute_partner_balance(partner_id, &[inv], &[payment]);

    // paid=400, outstanding=600
    assert!((balance.paid - 400.0).abs() < 0.01);
    assert!((balance.outstanding - 600.0).abs() < 0.01, "outstanding={}", balance.outstanding);
    assert_eq!(balance.direction, BalanceDirection::Receivable);
}

#[test]
fn test_partner_balance_fully_paid_invoice() {
    let partner_id = Uuid::new_v4();
    let inv = make_invoice(partner_id, 500.0, InvoiceState::Paid);
    let payment = make_payment(inv.id, 500.0);

    let balance = compute_partner_balance(partner_id, &[inv], &[payment]);

    // Fully paid
    assert!((balance.outstanding - 0.0).abs() < 0.01);
    assert_eq!(balance.direction, BalanceDirection::None);
}

#[test]
fn test_partner_balance_multiple_invoices() {
    let partner_id = Uuid::new_v4();
    let inv1 = make_invoice(partner_id, 800.0, InvoiceState::Posted);
    let inv2 = make_invoice(partner_id, 1200.0, InvoiceState::Posted);
    let payment = make_payment(inv1.id, 800.0);

    let balance = compute_partner_balance(partner_id, &[inv1, inv2], &[payment]);

    // inv1 fully paid, inv2 still open → outstanding=1200
    assert!((balance.outstanding - 1200.0).abs() < 0.01, "outstanding={}", balance.outstanding);
    assert_eq!(balance.invoice_count, 2);
    assert_eq!(balance.direction, BalanceDirection::Receivable);
}

#[test]
fn test_partner_balance_ignores_draft_invoices() {
    let partner_id = Uuid::new_v4();
    // Draft invoices should NOT contribute to balance
    let inv_draft = make_invoice(partner_id, 500.0, InvoiceState::Draft);
    let inv_posted = make_invoice(partner_id, 300.0, InvoiceState::Posted);

    let balance = compute_partner_balance(partner_id, &[inv_draft, inv_posted], &[]);

    // Only posted invoice counts
    assert!((balance.outstanding - 300.0).abs() < 0.01, "outstanding={}", balance.outstanding);
    assert_eq!(balance.invoice_count, 1); // only the posted one
}
